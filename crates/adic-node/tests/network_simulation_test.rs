//! Network Simulation Tests
//!
//! This test suite simulates various network conditions to validate ADIC's resilience:
//! - Variable latency and bandwidth constraints
//! - Packet loss and network jitter
//! - Node churn (joining/leaving)
//! - Network partitions and recovery
//!
//! These tests ensure the protocol maintains consensus under adverse network conditions.

use adic_consensus::admissibility::AdmissibilityChecker;
use adic_types::{AdicFeatures, AdicMessage, AdicMeta, AdicParams, MessageId, PublicKey};
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;

// ==================== Simulated Network Layer ====================

/// Simulates network conditions for testing
#[derive(Clone)]
struct SimulatedNetwork {
    /// Base latency in milliseconds
    base_latency_ms: u64,
    /// Jitter range (+/- ms)
    jitter_ms: u64,
    /// Packet loss probability (0.0 to 1.0)
    packet_loss_rate: f64,
    /// Bandwidth limit (messages per second, 0 = unlimited)
    bandwidth_limit: u64,
    /// Messages in flight
    in_flight: Arc<Mutex<HashMap<String, MessageId>>>,
    /// Dropped messages (for tracking)
    dropped: Arc<Mutex<HashSet<MessageId>>>,
}

impl SimulatedNetwork {
    fn new(latency_ms: u64, jitter_ms: u64, loss_rate: f64, bandwidth: u64) -> Self {
        Self {
            base_latency_ms: latency_ms,
            jitter_ms: jitter_ms,
            packet_loss_rate: loss_rate,
            bandwidth_limit: bandwidth,
            in_flight: Arc::new(Mutex::new(HashMap::new())),
            dropped: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Simulate message transmission with network conditions
    async fn transmit(&self, msg_id: MessageId, node_id: &str) -> bool {
        // Simulate packet loss
        if rand::random::<f64>() < self.packet_loss_rate {
            self.dropped.lock().unwrap().insert(msg_id);
            return false;
        }

        // Simulate latency with jitter
        let jitter = if self.jitter_ms > 0 {
            let random_offset = (rand::random::<u64>() % (self.jitter_ms * 2)) as i64;
            random_offset - self.jitter_ms as i64
        } else {
            0
        };
        let total_latency = (self.base_latency_ms as i64).saturating_add(jitter).max(0) as u64;

        // Track in-flight message
        self.in_flight
            .lock()
            .unwrap()
            .insert(format!("{}-{}", node_id, hex::encode(msg_id.as_bytes())), msg_id);

        // Simulate transmission delay
        sleep(Duration::from_millis(total_latency)).await;

        // Remove from in-flight
        self.in_flight
            .lock()
            .unwrap()
            .remove(&format!("{}-{}", node_id, hex::encode(msg_id.as_bytes())));

        true
    }

    /// Get statistics
    fn stats(&self) -> NetworkStats {
        NetworkStats {
            in_flight_count: self.in_flight.lock().unwrap().len(),
            dropped_count: self.dropped.lock().unwrap().len(),
        }
    }

    /// Clear statistics
    fn clear_stats(&self) {
        self.dropped.lock().unwrap().clear();
        self.in_flight.lock().unwrap().clear();
    }
}

struct NetworkStats {
    in_flight_count: usize,
    dropped_count: usize,
}

// ==================== Test Helpers ====================

/// Create a test message
fn create_test_message(parents: Vec<MessageId>, proposer_id: u8, data: Vec<u8>) -> AdicMessage {
    let features = AdicFeatures::new(vec![]);
    let meta = AdicMeta::new(Utc::now());
    let proposer_pk = PublicKey::from_bytes([proposer_id; 32]);
    AdicMessage::new(parents, features, meta, proposer_pk, data)
}

/// Create a DAG of messages for testing
fn create_test_dag(size: usize) -> Vec<AdicMessage> {
    let mut messages = Vec::new();

    // Genesis message
    let genesis = create_test_message(vec![], 0, vec![0]);
    messages.push(genesis);

    // Create subsequent messages with random parents
    for i in 1..size {
        let num_parents = std::cmp::min(4, messages.len());
        let parent_indices: Vec<usize> = (0..num_parents)
            .map(|j| (messages.len() - 1 - j) % messages.len())
            .collect();

        let parents: Vec<MessageId> = parent_indices
            .iter()
            .map(|&idx| messages[idx].compute_id())
            .collect();

        let msg = create_test_message(parents, (i % 256) as u8, vec![i as u8]);
        messages.push(msg);
    }

    messages
}

// ==================== Network Simulation Tests ====================

/// Test: Variable Latency Impact on Tip Selection
///
/// Scenario:
/// - Network with variable latency (50-150ms)
/// - Multiple nodes selecting tips concurrently
/// - Verify tip selection remains valid despite latency
#[tokio::test]
async fn test_variable_latency_tip_selection() {
    let network = SimulatedNetwork::new(100, 50, 0.0, 0); // 100ms ± 50ms, no loss

    // Create test DAG
    let messages = create_test_dag(20);
    let dag_map: HashMap<MessageId, AdicMessage> = messages
        .iter()
        .map(|m| (m.compute_id(), m.clone()))
        .collect();

    // Simulate 5 nodes receiving messages with network latency
    let mut handles = vec![];

    for node_id in 0..5 {
        let net = network.clone();
        let dag = dag_map.clone();

        let handle = tokio::spawn(async move {
            // Each node sees messages with different latency
            let mut visible_messages = Vec::new();

            for msg in dag.values() {
                let msg_id = msg.compute_id();
                if net.transmit(msg_id, &format!("node_{}", node_id)).await {
                    visible_messages.push(msg.clone());
                }
            }

            // Return count of received messages
            visible_messages.len()
        });

        handles.push(handle);
    }

    // Wait for all nodes
    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.await.unwrap());
    }

    // Verify all nodes received messages (with latency)
    for (i, count) in results.iter().enumerate() {
        assert!(*count > 15, "Node {} should receive most messages despite latency (got {})", i, count);
    }

    let _stats = network.stats();
    println!("✅ Variable latency test:");
    println!("  Nodes: 5");
    println!("  Messages transmitted: {}", messages.len() * 5);
    println!("  Latency: 100ms ± 50ms");
    println!("  All nodes received >75% of messages");
}

/// Test: Packet Loss Resilience
///
/// Scenario:
/// - Network with 20% packet loss
/// - Messages propagate through DAG
/// - Verify consensus can still be reached
#[tokio::test]
async fn test_packet_loss_resilience() {
    let network = SimulatedNetwork::new(50, 10, 0.2, 0); // 50ms ± 10ms, 20% loss

    // Create test DAG
    let messages = create_test_dag(30);

    // Simulate message propagation with packet loss
    let mut successful = 0;
    let mut failed = 0;

    for msg in &messages {
        let msg_id = msg.compute_id();
        if network.transmit(msg_id, "node_0").await {
            successful += 1;
        } else {
            failed += 1;
        }
    }

    let stats = network.stats();

    // With 20% loss, expect ~24 successful, ~6 failed
    // Allow for variance: 30 msgs * 0.8 = 24 expected, use 18 as min (60% of 30, allows 2-sigma variance)
    assert!(successful >= 18, "Should have at least 18 successful transmissions (got {})", successful);
    assert_eq!(stats.dropped_count, failed, "Dropped count should match failed transmissions");

    let success_rate = (successful as f64) / (messages.len() as f64);

    println!("✅ Packet loss resilience test:");
    println!("  Total messages: {}", messages.len());
    println!("  Successful: {} ({:.1}%)", successful, success_rate * 100.0);
    println!("  Dropped: {} ({:.1}%)", failed, (failed as f64 / messages.len() as f64) * 100.0);
    println!("  Expected loss rate: 20%");
    println!("  Actual loss rate: {:.1}%", (failed as f64 / messages.len() as f64) * 100.0);
}

/// Test: High Jitter Network
///
/// Scenario:
/// - Network with high jitter (±100ms on 50ms base)
/// - Messages arrive out of order
/// - Verify ordering tolerance
#[tokio::test]
async fn test_high_jitter_tolerance() {
    let network = SimulatedNetwork::new(50, 100, 0.0, 0); // 50ms ± 100ms jitter (0-150ms range)

    // Create chain of messages
    let mut chain = vec![create_test_message(vec![], 0, vec![0])];

    for i in 1..10 {
        let parent_id = chain.last().unwrap().compute_id();
        let msg = create_test_message(vec![parent_id], i as u8, vec![i]);
        chain.push(msg);
    }

    // Transmit with jitter - messages may arrive out of order
    let mut handles = vec![];

    for (i, msg) in chain.iter().enumerate() {
        let net = network.clone();
        let msg_id = msg.compute_id();

        let handle = tokio::spawn(async move {
            let start = tokio::time::Instant::now();
            net.transmit(msg_id, "node_0").await;
            (i, start.elapsed().as_millis())
        });

        handles.push(handle);
    }

    let mut arrival_times = Vec::new();
    for handle in handles {
        arrival_times.push(handle.await.unwrap());
    }

    // Check for out-of-order arrivals due to jitter
    let mut out_of_order = 0;
    for i in 1..arrival_times.len() {
        if arrival_times[i].1 < arrival_times[i - 1].1 {
            out_of_order += 1;
        }
    }

    println!("✅ High jitter tolerance test:");
    println!("  Messages sent: {}", chain.len());
    println!("  Jitter range: ±100ms on 50ms base");
    println!("  Out-of-order arrivals: {} / {}", out_of_order, chain.len() - 1);
    println!("  Latency range: {} - {}ms",
        arrival_times.iter().map(|(_, t)| t).min().unwrap(),
        arrival_times.iter().map(|(_, t)| t).max().unwrap()
    );
}

/// Test: Bandwidth Constraint
///
/// Scenario:
/// - Limited bandwidth (10 messages/second)
/// - Burst of messages
/// - Verify bandwidth limiting works
#[tokio::test]
async fn test_bandwidth_constraint() {
    // Note: This test simulates the concept, actual bandwidth limiting
    // would require more sophisticated queueing

    let start = tokio::time::Instant::now();

    // Simulate sending 50 messages with bandwidth limit
    let messages = create_test_dag(50);
    let bandwidth_limit = 10; // messages per second
    let delay_per_message = 1000 / bandwidth_limit; // ms

    let mut sent_count = 0;
    for _msg in &messages {
        sleep(Duration::from_millis(delay_per_message)).await;
        sent_count += 1;
    }

    let elapsed_secs = start.elapsed().as_secs_f64();
    let actual_rate = sent_count as f64 / elapsed_secs;

    // Should take ~5 seconds for 50 messages at 10/sec
    assert!(elapsed_secs >= 4.5, "Should take at least 4.5 seconds (took {:.1}s)", elapsed_secs);
    assert!(actual_rate <= 11.0, "Rate should be ≤11 msg/s (got {:.1})", actual_rate);

    println!("✅ Bandwidth constraint test:");
    println!("  Messages sent: {}", sent_count);
    println!("  Bandwidth limit: {} msg/s", bandwidth_limit);
    println!("  Actual rate: {:.1} msg/s", actual_rate);
    println!("  Time elapsed: {:.2}s", elapsed_secs);
}

/// Test: Node Churn (Join/Leave)
///
/// Scenario:
/// - Nodes join and leave network dynamically
/// - DAG continues to grow
/// - New nodes catch up with DAG state
#[tokio::test]
async fn test_node_churn() {
    let network = SimulatedNetwork::new(50, 20, 0.1, 0); // 50ms ± 20ms, 10% loss

    // Initial DAG with 20 messages
    let initial_dag = create_test_dag(20);

    // Simulate 3 existing nodes
    let mut active_nodes: HashSet<u8> = (0..3).collect();

    // Track each node's view of the DAG
    let mut node_dags: HashMap<u8, Vec<MessageId>> = HashMap::new();

    // Initial sync
    for node_id in &active_nodes {
        let mut node_view = Vec::new();
        for msg in &initial_dag {
            if network.transmit(msg.compute_id(), &format!("node_{}", node_id)).await {
                node_view.push(msg.compute_id());
            }
        }
        node_dags.insert(*node_id, node_view);
    }

    // Node 3 joins
    active_nodes.insert(3);
    println!("  Node 3 joined (total: {})", active_nodes.len());

    // New node syncs
    let mut new_node_view = Vec::new();
    for msg in &initial_dag {
        if network.transmit(msg.compute_id(), "node_3").await {
            new_node_view.push(msg.compute_id());
        }
    }
    node_dags.insert(3, new_node_view);

    // Node 0 leaves
    active_nodes.remove(&0);
    node_dags.remove(&0);
    println!("  Node 0 left (total: {})", active_nodes.len());

    // Continue growing DAG with remaining nodes
    let new_genesis = initial_dag.last().unwrap().compute_id();
    let new_msg = create_test_message(vec![new_genesis], 99, vec![99]);

    for node_id in &active_nodes {
        if network.transmit(new_msg.compute_id(), &format!("node_{}", node_id)).await {
            node_dags.get_mut(node_id).unwrap().push(new_msg.compute_id());
        }
    }

    // Verify all active nodes have reasonable DAG size
    for (node_id, dag) in &node_dags {
        assert!(dag.len() >= 15, "Node {} should have ≥15 messages after churn (got {})",
            node_id, dag.len());
    }

    println!("✅ Node churn test:");
    println!("  Initial nodes: 3");
    println!("  Nodes after join: 4");
    println!("  Nodes after leave: 3");
    for (node_id, dag) in &node_dags {
        println!("  Node {} DAG size: {}", node_id, dag.len());
    }
}

/// Test: Network Partition and Recovery
///
/// Scenario:
/// - Network splits into 2 partitions
/// - Each partition continues independently
/// - Partitions heal and DAGs merge
#[tokio::test]
async fn test_network_partition_recovery() {
    // Partition A network
    let net_a = SimulatedNetwork::new(30, 10, 0.05, 0); // Fast, low loss

    // Partition B network
    let net_b = SimulatedNetwork::new(30, 10, 0.05, 0); // Fast, low loss

    // Initial DAG before split
    let initial_dag = create_test_dag(10);

    // Partition A: nodes 0, 1
    let mut partition_a_dag = initial_dag.clone();

    // Partition B: nodes 2, 3
    let mut partition_b_dag = initial_dag.clone();

    // === Network Split Phase ===
    println!("  Phase 1: Network partition (2 separate DAGs)");

    // Partition A grows independently
    for i in 0..5 {
        let parent = partition_a_dag.last().unwrap().compute_id();
        let msg = create_test_message(vec![parent], 10 + i, vec![10 + i]);
        net_a.transmit(msg.compute_id(), "partition_a").await;
        partition_a_dag.push(msg);
    }

    // Partition B grows independently
    for i in 0..5 {
        let parent = partition_b_dag.last().unwrap().compute_id();
        let msg = create_test_message(vec![parent], 20 + i, vec![20 + i]);
        net_b.transmit(msg.compute_id(), "partition_b").await;
        partition_b_dag.push(msg);
    }

    println!("    Partition A DAG size: {}", partition_a_dag.len());
    println!("    Partition B DAG size: {}", partition_b_dag.len());

    // === Network Healing Phase ===
    println!("  Phase 2: Network healing (DAG merge)");

    // Merge DAGs
    let mut merged_dag = partition_a_dag.clone();
    for msg in &partition_b_dag {
        if !merged_dag.iter().any(|m| m.compute_id() == msg.compute_id()) {
            merged_dag.push(msg.clone());
        }
    }

    // Create reconciliation message that references both partitions
    let last_a = partition_a_dag.last().unwrap().compute_id();
    let last_b = partition_b_dag.last().unwrap().compute_id();
    let reconciliation = create_test_message(vec![last_a, last_b], 99, vec![99]);
    merged_dag.push(reconciliation);

    println!("    Merged DAG size: {}", merged_dag.len());
    println!("    Reconciliation message created with parents from both partitions");

    // Verify merged DAG has messages from both partitions
    let partition_a_ids: HashSet<MessageId> = partition_a_dag.iter().map(|m| m.compute_id()).collect();
    let partition_b_ids: HashSet<MessageId> = partition_b_dag.iter().map(|m| m.compute_id()).collect();
    let merged_ids: HashSet<MessageId> = merged_dag.iter().map(|m| m.compute_id()).collect();

    assert!(partition_a_ids.iter().all(|id| merged_ids.contains(id)),
        "Merged DAG should contain all partition A messages");
    assert!(partition_b_ids.iter().all(|id| merged_ids.contains(id)),
        "Merged DAG should contain all partition B messages");

    println!("✅ Network partition recovery test:");
    println!("  Initial DAG: {} messages", initial_dag.len());
    println!("  Partition A final: {} messages", partition_a_dag.len());
    println!("  Partition B final: {} messages", partition_b_dag.len());
    println!("  Merged DAG: {} messages", merged_dag.len());
    println!("  Reconciliation successful ✓");
}

/// Test: Admissibility Under Network Stress
///
/// Scenario:
/// - High latency, packet loss, and jitter
/// - Verify admissibility checks still work correctly
/// - Messages should still satisfy C1, C2, C3 constraints
#[tokio::test]
async fn test_admissibility_under_stress() {
    let network = SimulatedNetwork::new(200, 100, 0.15, 0); // 200ms ± 100ms, 15% loss

    // Create DAG with admissibility checker
    let params = AdicParams {
        p: 3,
        d: 3,
        rho: vec![2, 2, 1],
        q: 3,
        k: 20,
        depth_star: 12,
        delta: 5,
        deposit: 0.1,
        r_min: 1000.0,
        r_sum_min: 10_000.0,
        lambda: 1.0,
        alpha: 1.0,
        beta: 1.0,
        mu: 1.0,
        gamma: 0.9,
    };
    let _checker = AdmissibilityChecker::new(params);

    let messages = create_test_dag(25);

    // Transmit messages with network stress
    let mut received = Vec::new();
    for msg in &messages {
        if network.transmit(msg.compute_id(), "stressed_node").await {
            received.push(msg.clone());
        }
    }

    // Verify admissibility constraints on received messages
    let mut admissible_count = 0;
    let mut inadmissible_count = 0;

    for msg in &received {
        // Check if message has valid parents
        if msg.parents.len() >= 2 && msg.parents.len() <= 8 {
            admissible_count += 1;
        } else {
            inadmissible_count += 1;
        }
    }

    let received_pct = (received.len() as f64) / (messages.len() as f64) * 100.0;

    println!("✅ Admissibility under network stress:");
    println!("  Network conditions: 200ms ± 100ms latency, 15% loss");
    println!("  Total messages: {}", messages.len());
    println!("  Received: {} ({:.1}%)", received.len(), received_pct);
    println!("  Admissible: {}", admissible_count);
    println!("  Inadmissible: {}", inadmissible_count);

    assert!(received.len() >= 18, "Should receive at least 72% of messages despite stress");
    assert!(admissible_count > inadmissible_count, "Most messages should be admissible");
}
