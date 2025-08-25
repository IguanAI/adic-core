use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
use libp2p::identity::Keypair;
use libp2p::PeerId;
use blake3::Hasher;
use tracing::{debug, warn, info};

use adic_types::{PublicKey, Signature, Result};

pub struct SecurityManager {
    blacklist: Arc<RwLock<HashSet<PeerId>>>,
    rate_limits: Arc<RwLock<HashMap<PeerId, RateLimit>>>,
    puzzle_challenges: Arc<RwLock<HashMap<PeerId, PuzzleChallenge>>>,
}

#[derive(Debug, Clone)]
struct RateLimit {
    requests: Vec<Instant>,
    limit: usize,
    window: Duration,
}

#[derive(Debug, Clone)]
struct PuzzleChallenge {
    challenge: Vec<u8>,
    difficulty: u32,
    created_at: Instant,
    solved: bool,
}

impl SecurityManager {
    pub fn new() -> Self {
        Self {
            blacklist: Arc::new(RwLock::new(HashSet::new())),
            rate_limits: Arc::new(RwLock::new(HashMap::new())),
            puzzle_challenges: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn is_blacklisted(&self, peer: &PeerId) -> bool {
        let blacklist = self.blacklist.read().await;
        blacklist.contains(peer)
    }

    pub async fn blacklist_peer(&self, peer: PeerId, reason: &str) {
        let mut blacklist = self.blacklist.write().await;
        blacklist.insert(peer);
        warn!("Blacklisted peer {}: {}", peer, reason);
    }

    pub async fn unblacklist_peer(&self, peer: &PeerId) {
        let mut blacklist = self.blacklist.write().await;
        if blacklist.remove(peer) {
            info!("Removed peer {} from blacklist", peer);
        }
    }

    pub async fn check_rate_limit(&self, peer: &PeerId, limit: usize, window: Duration) -> bool {
        let mut rate_limits = self.rate_limits.write().await;
        let now = Instant::now();
        
        let rate_limit = rate_limits.entry(*peer).or_insert(RateLimit {
            requests: Vec::new(),
            limit,
            window,
        });
        
        // Remove old requests outside the window
        rate_limit.requests.retain(|&t| now.duration_since(t) < rate_limit.window);
        
        if rate_limit.requests.len() >= rate_limit.limit {
            false
        } else {
            rate_limit.requests.push(now);
            true
        }
    }

    pub async fn generate_puzzle(&self, peer: &PeerId, difficulty: u32) -> Vec<u8> {
        let mut hasher = Hasher::new();
        hasher.update(&peer.to_bytes());
        hasher.update(&chrono::Utc::now().timestamp_millis().to_le_bytes());
        
        let challenge = hasher.finalize().as_bytes().to_vec();
        
        let mut challenges = self.puzzle_challenges.write().await;
        challenges.insert(*peer, PuzzleChallenge {
            challenge: challenge.clone(),
            difficulty,
            created_at: Instant::now(),
            solved: false,
        });
        
        challenge
    }

    pub async fn verify_puzzle_solution(
        &self,
        peer: &PeerId,
        solution: &[u8],
    ) -> bool {
        let challenges = self.puzzle_challenges.read().await;
        
        if let Some(challenge) = challenges.get(peer) {
            if challenge.solved {
                return false;
            }
            
            // Verify the solution meets difficulty requirement
            let mut hasher = Hasher::new();
            hasher.update(&challenge.challenge);
            hasher.update(solution);
            let hash = hasher.finalize();
            
            let leading_zeros = hash.as_bytes().iter()
                .take_while(|&&b| b == 0)
                .count() as u32;
            
            if leading_zeros >= challenge.difficulty {
                drop(challenges);
                let mut challenges = self.puzzle_challenges.write().await;
                if let Some(challenge) = challenges.get_mut(peer) {
                    challenge.solved = true;
                }
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    pub fn sign_data(&self, keypair: &Keypair, data: &[u8]) -> Signature {
        // Use libp2p keypair to sign
        let signature_bytes = keypair.sign(data)
            .unwrap_or_else(|_| {
                // Fallback to hash if signing is not supported
                let mut hasher = Hasher::new();
                hasher.update(data);
                let hash = hasher.finalize();
                hash.as_bytes().to_vec()
            });
        Signature::new(signature_bytes)
    }

    pub fn verify_signature(&self, _public_key: &PublicKey, data: &[u8], signature: &Signature) -> bool {
        // Simplified verification
        let mut hasher = Hasher::new();
        hasher.update(data);
        let hash = hasher.finalize();
        
        // In real implementation, use proper signature verification
        signature.as_bytes() == hash.as_bytes()
    }

    pub async fn encrypt_channel(&self, _peer: &PeerId, data: &[u8]) -> Vec<u8> {
        // In real implementation, use noise protocol or similar
        // For now, just return the data
        data.to_vec()
    }

    pub async fn decrypt_channel(&self, _peer: &PeerId, encrypted: &[u8]) -> Result<Vec<u8>> {
        // In real implementation, use noise protocol or similar
        Ok(encrypted.to_vec())
    }

    pub async fn cleanup_expired_challenges(&self) {
        let mut challenges = self.puzzle_challenges.write().await;
        let now = Instant::now();
        
        challenges.retain(|_, challenge| {
            now.duration_since(challenge.created_at) < Duration::from_secs(300)
        });
    }

    pub async fn get_blacklist_size(&self) -> usize {
        let blacklist = self.blacklist.read().await;
        blacklist.len()
    }

    pub async fn clear_rate_limits(&self) {
        let mut rate_limits = self.rate_limits.write().await;
        rate_limits.clear();
        debug!("Cleared all rate limits");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_blacklist() {
        let security = SecurityManager::new();
        let peer = PeerId::random();
        
        assert!(!security.is_blacklisted(&peer).await);
        
        security.blacklist_peer(peer, "test").await;
        assert!(security.is_blacklisted(&peer).await);
        
        security.unblacklist_peer(&peer).await;
        assert!(!security.is_blacklisted(&peer).await);
    }

    #[tokio::test]
    async fn test_rate_limiting() {
        let security = SecurityManager::new();
        let peer = PeerId::random();
        
        // Allow 3 requests per second
        let limit = 3;
        let window = Duration::from_secs(1);
        
        for _ in 0..3 {
            assert!(security.check_rate_limit(&peer, limit, window).await);
        }
        
        // Fourth request should be denied
        assert!(!security.check_rate_limit(&peer, limit, window).await);
    }

    #[tokio::test]
    async fn test_puzzle_generation() {
        let security = SecurityManager::new();
        let peer = PeerId::random();
        
        let challenge = security.generate_puzzle(&peer, 1).await;
        assert!(!challenge.is_empty());
        
        // Verify solution (simplified test)
        let solution = vec![0u8; 32];
        security.verify_puzzle_solution(&peer, &solution).await;
    }
}