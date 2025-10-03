use adic_storage::StorageEngine;
use adic_types::{MessageId, Result};
use std::collections::{HashMap, HashSet, VecDeque};

use adic_consensus::ReputationTracker;

/// Result of k-core analysis
#[derive(Debug, Clone)]
pub struct KCoreResult {
    pub is_final: bool,
    pub core_size: usize,
    pub depth: u32,
    pub total_reputation: f64,
    pub distinct_balls: HashMap<u32, usize>,
    pub kcore_root: Option<MessageId>,
}

impl KCoreResult {
    pub fn not_final() -> Self {
        Self {
            is_final: false,
            core_size: 0,
            depth: 0,
            total_reputation: 0.0,
            distinct_balls: HashMap::new(),
            kcore_root: None,
        }
    }
}

/// k-core analyzer for finality determination
#[derive(Debug, Clone)]
pub struct KCoreAnalyzer {
    k: usize,
    min_depth: u32,
    min_diversity: usize,
    min_reputation: f64,
    rho: Vec<u32>,
}

impl KCoreAnalyzer {
    pub fn new(
        k: usize,
        min_depth: u32,
        min_diversity: usize,
        min_reputation: f64,
        rho: Vec<u32>,
    ) -> Self {
        Self {
            k,
            min_depth,
            min_diversity,
            min_reputation,
            rho,
        }
    }

    /// Analyze the k-core starting from a message
    pub async fn analyze(
        &self,
        root: MessageId,
        storage: &StorageEngine,
        reputation: &ReputationTracker,
    ) -> Result<KCoreResult> {
        // Build forward cone from root
        let forward_cone = self.build_forward_cone(root, storage).await?;

        if forward_cone.is_empty() {
            return Ok(KCoreResult::not_final());
        }

        // Compute k-core of the forward cone
        let kcore = self.compute_kcore(&forward_cone, storage).await?;

        if kcore.is_empty() {
            return Ok(KCoreResult::not_final());
        }

        // Check finality conditions
        let depth = self.compute_depth(&kcore, root, storage).await;
        let (total_rep, distinct_balls) = self.compute_metrics(&kcore, storage, reputation).await?;

        let is_final = kcore.len() >= self.k
            && depth >= self.min_depth
            && distinct_balls
                .values()
                .all(|&count| count >= self.min_diversity)
            && total_rep >= self.min_reputation;

        Ok(KCoreResult {
            is_final,
            core_size: kcore.len(),
            depth,
            total_reputation: total_rep,
            distinct_balls,
            kcore_root: Some(root),
        })
    }

    /// Build the forward cone of descendants
    async fn build_forward_cone(
        &self,
        root: MessageId,
        storage: &StorageEngine,
    ) -> Result<HashSet<MessageId>> {
        let mut cone = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(root);
        cone.insert(root);

        while let Some(current) = queue.pop_front() {
            if let Ok(children) = storage.get_children(&current).await {
                for child in children {
                    if cone.insert(child) {
                        queue.push_back(child);
                    }
                }
            }
        }

        Ok(cone)
    }

    /// Compute k-core using iterative degree reduction
    async fn compute_kcore(
        &self,
        nodes: &HashSet<MessageId>,
        storage: &StorageEngine,
    ) -> Result<HashSet<MessageId>> {
        let mut core = nodes.clone();
        let mut degrees: HashMap<MessageId, usize> = HashMap::new();

        // Initialize degrees
        for &node in nodes {
            let mut degree = 0;
            if let Ok(parents) = storage.get_parents(&node).await {
                degree += parents.iter().filter(|p| nodes.contains(p)).count();
            }
            if let Ok(children) = storage.get_children(&node).await {
                degree += children.iter().filter(|c| nodes.contains(c)).count();
            }
            degrees.insert(node, degree);
        }

        // Iteratively remove nodes with degree < k
        let mut changed = true;
        while changed {
            changed = false;
            let to_remove: Vec<MessageId> = degrees
                .iter()
                .filter(|(id, &deg)| core.contains(id) && deg < self.k)
                .map(|(id, _)| *id)
                .collect();

            for node in to_remove {
                core.remove(&node);
                changed = true;

                // Update degrees of neighbors
                if let Ok(parents) = storage.get_parents(&node).await {
                    for parent in parents {
                        if let Some(deg) = degrees.get_mut(&parent) {
                            *deg = deg.saturating_sub(1);
                        }
                    }
                }
                if let Ok(children) = storage.get_children(&node).await {
                    for child in children {
                        if let Some(deg) = degrees.get_mut(&child) {
                            *deg = deg.saturating_sub(1);
                        }
                    }
                }
            }
        }

        Ok(core)
    }

    /// Compute depth of the k-core from root
    async fn compute_depth(
        &self,
        kcore: &HashSet<MessageId>,
        root: MessageId,
        storage: &StorageEngine,
    ) -> u32 {
        let mut max_depth = 0;
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();

        queue.push_back((root, 0));
        visited.insert(root);

        while let Some((current, depth)) = queue.pop_front() {
            max_depth = max_depth.max(depth);

            if let Ok(children) = storage.get_children(&current).await {
                for child in children {
                    if kcore.contains(&child) && visited.insert(child) {
                        queue.push_back((child, depth + 1));
                    }
                }
            }
        }

        max_depth
    }

    /// Compute reputation and diversity metrics
    async fn compute_metrics(
        &self,
        kcore: &HashSet<MessageId>,
        storage: &StorageEngine,
        reputation: &ReputationTracker,
    ) -> Result<(f64, HashMap<u32, usize>)> {
        let mut total_reputation = 0.0;
        let mut balls_per_axis: HashMap<u32, HashSet<Vec<u8>>> = HashMap::new();

        for &node in kcore {
            if let Ok(Some(msg)) = storage.get_message(&node).await {
                total_reputation += reputation.get_reputation(&msg.proposer_pk).await;

                for axis_phi in &msg.features.phi {
                    // Use radius from params for this axis
                    let axis_idx = axis_phi.axis.0 as usize;
                    let radius = self.rho.get(axis_idx).copied().unwrap_or(2) as usize;
                    let ball_id = axis_phi.qp_digits.ball_id(radius);
                    balls_per_axis
                        .entry(axis_phi.axis.0)
                        .or_default()
                        .insert(ball_id);
                }
            }
        }

        let distinct_balls: HashMap<u32, usize> = balls_per_axis
            .into_iter()
            .map(|(axis, balls)| (axis, balls.len()))
            .collect();

        Ok((total_reputation, distinct_balls))
    }
}
