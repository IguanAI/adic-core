use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, RwLock};
use tracing::{debug, info, warn};

use adic_types::{MessageId, PublicKey, Result};

#[derive(Debug, Clone)]
pub struct ConsensusConfig {
    pub message_timeout: Duration,
    pub vote_threshold: f64,
    pub max_pending_votes: usize,
    pub coordination_interval: Duration,
}

impl Default for ConsensusConfig {
    fn default() -> Self {
        Self {
            message_timeout: Duration::from_secs(5),
            vote_threshold: 0.67,
            max_pending_votes: 1000,
            coordination_interval: Duration::from_secs(2),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConsensusMessage {
    ParentProposal {
        proposer: PublicKey,
        message_id: MessageId,
        proposed_parents: Vec<MessageId>,
        features: Vec<u8>, // Serialized features
        timestamp: u64,
    },
    ParentVote {
        voter: PublicKey,
        proposal_id: MessageId,
        approved: bool,
        reason: Option<String>,
    },
    ConflictDetection {
        detector: PublicKey,
        conflict_set: Vec<MessageId>,
        evidence: Vec<u8>,
    },
    FinalityProposal {
        proposer: PublicKey,
        message_id: MessageId,
        k_core_value: u64,
        witnesses: Vec<PublicKey>,
    },
    FinalityAck {
        acknowledger: PublicKey,
        proposal_id: MessageId,
        accepted: bool,
    },
    CoordinationPing {
        sender: PublicKey,
        tips: Vec<MessageId>,
        local_time: u64,
    },
}

#[derive(Debug, Clone)]
struct ProposalState {
    proposal: ConsensusMessage,
    votes: HashMap<PublicKey, bool>,
    created_at: Instant,
    decided: bool,
}

pub struct ConsensusProtocol {
    config: ConsensusConfig,
    proposals: Arc<RwLock<HashMap<MessageId, ProposalState>>>,
    pending_decisions: Arc<RwLock<HashMap<MessageId, oneshot::Sender<bool>>>>,
    event_sender: mpsc::UnboundedSender<ConsensusEvent>,
    event_receiver: Arc<RwLock<mpsc::UnboundedReceiver<ConsensusEvent>>>,
}

#[derive(Debug, Clone)]
pub enum ConsensusEvent {
    ProposalReceived(MessageId, ConsensusMessage),
    VoteReceived(MessageId, PublicKey, bool),
    ProposalDecided(MessageId, bool),
    ConflictDetected(Vec<MessageId>),
    FinalityAchieved(MessageId, u64),
}

impl ConsensusProtocol {
    pub fn new(config: ConsensusConfig) -> Self {
        let (event_sender, event_receiver) = mpsc::unbounded_channel();

        Self {
            config,
            proposals: Arc::new(RwLock::new(HashMap::new())),
            pending_decisions: Arc::new(RwLock::new(HashMap::new())),
            event_sender,
            event_receiver: Arc::new(RwLock::new(event_receiver)),
        }
    }

    pub async fn propose_parents(
        &self,
        proposer: PublicKey,
        message_id: MessageId,
        parents: Vec<MessageId>,
        features: Vec<u8>,
    ) -> Result<oneshot::Receiver<bool>> {
        let proposal = ConsensusMessage::ParentProposal {
            proposer,
            message_id,
            proposed_parents: parents,
            features,
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
        };

        let (tx, rx) = oneshot::channel();

        let mut proposals = self.proposals.write().await;
        proposals.insert(
            message_id,
            ProposalState {
                proposal: proposal.clone(),
                votes: HashMap::new(),
                created_at: Instant::now(),
                decided: false,
            },
        );

        let mut pending = self.pending_decisions.write().await;
        pending.insert(message_id, tx);

        self.event_sender
            .send(ConsensusEvent::ProposalReceived(message_id, proposal))
            .ok();

        info!("Proposed parents for message {}", message_id);
        Ok(rx)
    }

    pub async fn vote_on_proposal(
        &self,
        voter: PublicKey,
        proposal_id: MessageId,
        approved: bool,
        _reason: Option<String>,
    ) -> Result<()> {
        let mut proposals = self.proposals.write().await;
        if let Some(proposal_state) = proposals.get_mut(&proposal_id) {
            if !proposal_state.decided {
                proposal_state.votes.insert(voter, approved);

                self.event_sender
                    .send(ConsensusEvent::VoteReceived(proposal_id, voter, approved))
                    .ok();

                // Check if we have enough votes to decide
                self.check_proposal_decision(proposal_id, proposal_state)
                    .await?;
            }
        }

        Ok(())
    }

    pub async fn get_proposal(&self, proposal_id: &MessageId) -> Option<ConsensusMessage> {
        let proposals = self.proposals.read().await;
        proposals
            .get(proposal_id)
            .map(|state| state.proposal.clone())
    }

    async fn check_proposal_decision(
        &self,
        proposal_id: MessageId,
        proposal_state: &mut ProposalState,
    ) -> Result<()> {
        let total_votes = proposal_state.votes.len();
        if total_votes == 0 {
            return Ok(());
        }

        let approved_votes = proposal_state.votes.values().filter(|&&v| v).count();

        let approval_ratio = approved_votes as f64 / total_votes as f64;

        if approval_ratio >= self.config.vote_threshold {
            proposal_state.decided = true;

            let mut pending = self.pending_decisions.write().await;
            if let Some(sender) = pending.remove(&proposal_id) {
                sender.send(true).ok();
            }

            self.event_sender
                .send(ConsensusEvent::ProposalDecided(proposal_id, true))
                .ok();
            info!(
                "Proposal {} approved with {:.1}% votes",
                proposal_id,
                approval_ratio * 100.0
            );
        } else if approval_ratio < (1.0 - self.config.vote_threshold) {
            proposal_state.decided = true;

            let mut pending = self.pending_decisions.write().await;
            if let Some(sender) = pending.remove(&proposal_id) {
                sender.send(false).ok();
            }

            self.event_sender
                .send(ConsensusEvent::ProposalDecided(proposal_id, false))
                .ok();
            info!(
                "Proposal {} rejected with {:.1}% votes",
                proposal_id,
                approval_ratio * 100.0
            );
        }

        Ok(())
    }

    pub async fn report_conflict(
        &self,
        detector: PublicKey,
        conflict_set: Vec<MessageId>,
        evidence: Vec<u8>,
    ) -> Result<()> {
        // In a real implementation, would broadcast the conflict detection message
        let _message = ConsensusMessage::ConflictDetection {
            detector,
            conflict_set: conflict_set.clone(),
            evidence,
        };

        self.event_sender
            .send(ConsensusEvent::ConflictDetected(conflict_set))
            .ok();

        info!("Conflict detected by {:?}", detector);
        Ok(())
    }

    pub async fn propose_finality(
        &self,
        proposer: PublicKey,
        message_id: MessageId,
        k_core_value: u64,
        witnesses: Vec<PublicKey>,
    ) -> Result<()> {
        // In a real implementation, would broadcast the finality proposal
        let _proposal = ConsensusMessage::FinalityProposal {
            proposer,
            message_id,
            k_core_value,
            witnesses,
        };

        self.event_sender
            .send(ConsensusEvent::FinalityAchieved(message_id, k_core_value))
            .ok();

        info!(
            "Finality proposed for message {} with k-core {}",
            message_id, k_core_value
        );
        Ok(())
    }

    pub async fn acknowledge_finality(
        &self,
        acknowledger: PublicKey,
        proposal_id: MessageId,
        accepted: bool,
    ) -> Result<()> {
        let _ack = ConsensusMessage::FinalityAck {
            acknowledger,
            proposal_id,
            accepted,
        };

        debug!("Finality acknowledgment for {} : {}", proposal_id, accepted);
        Ok(())
    }

    pub async fn send_coordination_ping(
        &self,
        sender: PublicKey,
        tips: Vec<MessageId>,
    ) -> Result<()> {
        let tip_count = tips.len();
        let _ping = ConsensusMessage::CoordinationPing {
            sender,
            tips,
            local_time: chrono::Utc::now().timestamp_millis() as u64,
        };

        debug!("Coordination ping sent with {} tips", tip_count);
        Ok(())
    }

    pub async fn cleanup_old_proposals(&self) {
        let mut proposals = self.proposals.write().await;
        let now = Instant::now();
        let timeout = self.config.message_timeout;

        let expired: Vec<MessageId> = proposals
            .iter()
            .filter(|(_, state)| !state.decided && now.duration_since(state.created_at) > timeout)
            .map(|(id, _)| *id)
            .collect();

        for proposal_id in expired {
            proposals.remove(&proposal_id);

            let mut pending = self.pending_decisions.write().await;
            if let Some(sender) = pending.remove(&proposal_id) {
                sender.send(false).ok();
            }

            warn!("Proposal {} expired", proposal_id);
        }
    }

    pub async fn pending_proposal_count(&self) -> usize {
        let proposals = self.proposals.read().await;
        proposals.iter().filter(|(_, state)| !state.decided).count()
    }

    pub async fn get_proposal_status(&self, proposal_id: &MessageId) -> Option<(usize, bool)> {
        let proposals = self.proposals.read().await;
        proposals
            .get(proposal_id)
            .map(|state| (state.votes.len(), state.decided))
    }

    pub fn event_stream(&self) -> Arc<RwLock<mpsc::UnboundedReceiver<ConsensusEvent>>> {
        self.event_receiver.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_consensus_protocol_creation() {
        let config = ConsensusConfig::default();
        let protocol = ConsensusProtocol::new(config);

        assert_eq!(protocol.pending_proposal_count().await, 0);
    }

    #[tokio::test]
    async fn test_parent_proposal() {
        let config = ConsensusConfig::default();
        let protocol = ConsensusProtocol::new(config);

        let proposer = PublicKey::from_bytes([1; 32]);
        let message_id = MessageId::new(b"test");
        let parents = vec![MessageId::new(b"parent1"), MessageId::new(b"parent2")];

        let _rx = protocol
            .propose_parents(proposer, message_id, parents, vec![])
            .await
            .unwrap();

        assert_eq!(protocol.pending_proposal_count().await, 1);
    }

    #[tokio::test]
    async fn test_voting() {
        let config = ConsensusConfig::default();
        let protocol = ConsensusProtocol::new(config);

        let proposer = PublicKey::from_bytes([1; 32]);
        let message_id = MessageId::new(b"test");
        let parents = vec![MessageId::new(b"parent1")];

        let _rx = protocol
            .propose_parents(proposer, message_id, parents, vec![])
            .await
            .unwrap();

        // Vote on the proposal
        for i in 0..10 {
            let voter = PublicKey::from_bytes([i; 32]);
            protocol
                .vote_on_proposal(voter, message_id, true, None)
                .await
                .unwrap();
        }

        // Check if proposal was decided
        let status = protocol.get_proposal_status(&message_id).await;
        assert!(status.is_some());
    }
}
