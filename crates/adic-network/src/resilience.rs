use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
use tracing::{debug, warn, info};

use libp2p::PeerId;

#[derive(Debug, Clone)]
pub struct ResilienceConfig {
    pub circuit_breaker_threshold: usize,
    pub circuit_breaker_timeout: Duration,
    pub backoff_base: Duration,
    pub backoff_max: Duration,
    pub health_check_interval: Duration,
}

impl Default for ResilienceConfig {
    fn default() -> Self {
        Self {
            circuit_breaker_threshold: 3,
            circuit_breaker_timeout: Duration::from_secs(60),
            backoff_base: Duration::from_secs(1),
            backoff_max: Duration::from_secs(300),
            health_check_interval: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone)]
struct CircuitBreaker {
    failures: usize,
    last_failure: Option<Instant>,
    state: CircuitState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

pub struct NetworkResilience {
    config: ResilienceConfig,
    circuit_breakers: Arc<RwLock<HashMap<PeerId, CircuitBreaker>>>,
    backoff_states: Arc<RwLock<HashMap<PeerId, BackoffState>>>,
}

#[derive(Debug, Clone)]
struct BackoffState {
    attempt: usize,
    next_retry: Instant,
}

impl NetworkResilience {
    pub fn new(config: ResilienceConfig) -> Self {
        Self {
            config,
            circuit_breakers: Arc::new(RwLock::new(HashMap::new())),
            backoff_states: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn record_failure(&self, peer: &PeerId) {
        let mut breakers = self.circuit_breakers.write().await;
        let breaker = breakers.entry(*peer).or_insert(CircuitBreaker {
            failures: 0,
            last_failure: None,
            state: CircuitState::Closed,
        });

        breaker.failures += 1;
        breaker.last_failure = Some(Instant::now());

        if breaker.failures >= self.config.circuit_breaker_threshold {
            breaker.state = CircuitState::Open;
            warn!("Circuit breaker opened for peer {}", peer);
        }
    }

    pub async fn record_success(&self, peer: &PeerId) {
        let mut breakers = self.circuit_breakers.write().await;
        if let Some(breaker) = breakers.get_mut(peer) {
            breaker.failures = 0;
            if breaker.state == CircuitState::HalfOpen {
                breaker.state = CircuitState::Closed;
                info!("Circuit breaker closed for peer {}", peer);
            }
        }
    }

    pub async fn can_connect(&self, peer: &PeerId) -> bool {
        let breakers = self.circuit_breakers.read().await;
        
        if let Some(breaker) = breakers.get(peer) {
            match breaker.state {
                CircuitState::Closed => true,
                CircuitState::Open => {
                    if let Some(last_failure) = breaker.last_failure {
                        if last_failure.elapsed() > self.config.circuit_breaker_timeout {
                            // Move to half-open state
                            drop(breakers);
                            let mut breakers = self.circuit_breakers.write().await;
                            if let Some(breaker) = breakers.get_mut(peer) {
                                breaker.state = CircuitState::HalfOpen;
                            }
                            true
                        } else {
                            false
                        }
                    } else {
                        true
                    }
                }
                CircuitState::HalfOpen => true,
            }
        } else {
            true
        }
    }

    pub async fn calculate_backoff(&self, peer: &PeerId) -> Duration {
        let mut backoffs = self.backoff_states.write().await;
        let state = backoffs.entry(*peer).or_insert(BackoffState {
            attempt: 0,
            next_retry: Instant::now(),
        });

        state.attempt += 1;
        
        let backoff = self.config.backoff_base * 2u32.pow(state.attempt.min(10) as u32);
        let backoff = backoff.min(self.config.backoff_max);
        
        state.next_retry = Instant::now() + backoff;
        backoff
    }

    pub async fn reset_backoff(&self, peer: &PeerId) {
        let mut backoffs = self.backoff_states.write().await;
        backoffs.remove(peer);
    }

    pub async fn check_network_health(&self) {
        let breakers = self.circuit_breakers.read().await;
        let open_count = breakers.values()
            .filter(|b| b.state == CircuitState::Open)
            .count();
        
        if open_count > 0 {
            warn!("{} circuit breakers are open", open_count);
        }
        
        debug!("Network health check: {} total peers monitored", breakers.len());
    }

    pub async fn cleanup_old_states(&self) {
        let mut breakers = self.circuit_breakers.write().await;
        
        breakers.retain(|_, breaker| {
            if let Some(last_failure) = breaker.last_failure {
                last_failure.elapsed() < Duration::from_secs(3600)
            } else {
                false
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_circuit_breaker() {
        let resilience = NetworkResilience::new(Default::default());
        let peer = PeerId::random();
        
        assert!(resilience.can_connect(&peer).await);
        
        // Record failures
        for _ in 0..3 {
            resilience.record_failure(&peer).await;
        }
        
        // Circuit should be open
        assert!(!resilience.can_connect(&peer).await);
    }

    #[tokio::test]
    async fn test_backoff() {
        let resilience = NetworkResilience::new(Default::default());
        let peer = PeerId::random();
        
        let backoff1 = resilience.calculate_backoff(&peer).await;
        let backoff2 = resilience.calculate_backoff(&peer).await;
        
        assert!(backoff2 > backoff1);
        
        resilience.reset_backoff(&peer).await;
        let backoff3 = resilience.calculate_backoff(&peer).await;
        
        assert_eq!(backoff3, backoff1);
    }
}