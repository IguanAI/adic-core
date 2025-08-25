use adic_types::{MessageId, Result};
use std::collections::{HashMap, HashSet, VecDeque};

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
}

impl KCoreAnalyzer {
    pub fn new(k: usize, min_depth: u32, min_diversity: usize, min_reputation: f64) -> Self {
        Self {
            k,
            min_depth,
            min_diversity,
            min_reputation,
        }
    }

    /// Analyze the k-core starting from a message
    pub fn analyze(
        &self,
        root: MessageId,
        graph: &MessageGraph,
    ) -> Result<KCoreResult> {
        // Build forward cone from root
        let forward_cone = self.build_forward_cone(root, graph)?;
        
        if forward_cone.is_empty() {
            return Ok(KCoreResult::not_final());
        }

        // Compute k-core of the forward cone
        let kcore = self.compute_kcore(&forward_cone, graph)?;
        
        if kcore.is_empty() {
            return Ok(KCoreResult::not_final());
        }

        // Check finality conditions
        let depth = self.compute_depth(&kcore, root, graph);
        let (total_rep, distinct_balls) = self.compute_metrics(&kcore, graph)?;
        
        let is_final = kcore.len() >= self.k
            && depth >= self.min_depth
            && distinct_balls.values().all(|&count| count >= self.min_diversity)
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
    fn build_forward_cone(
        &self,
        root: MessageId,
        graph: &MessageGraph,
    ) -> Result<HashSet<MessageId>> {
        let mut cone = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(root);
        cone.insert(root);

        while let Some(current) = queue.pop_front() {
            if let Some(children) = graph.get_children(&current) {
                for child in children {
                    if cone.insert(*child) {
                        queue.push_back(*child);
                    }
                }
            }
        }

        Ok(cone)
    }

    /// Compute k-core using iterative degree reduction
    fn compute_kcore(
        &self,
        nodes: &HashSet<MessageId>,
        graph: &MessageGraph,
    ) -> Result<HashSet<MessageId>> {
        let mut core = nodes.clone();
        let mut degrees: HashMap<MessageId, usize> = HashMap::new();
        
        // Initialize degrees
        for &node in nodes {
            let mut degree = 0;
            if let Some(parents) = graph.get_parents(&node) {
                degree += parents.iter().filter(|p| nodes.contains(p)).count();
            }
            if let Some(children) = graph.get_children(&node) {
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
                if let Some(parents) = graph.get_parents(&node) {
                    for parent in parents {
                        if let Some(deg) = degrees.get_mut(parent) {
                            *deg = deg.saturating_sub(1);
                        }
                    }
                }
                if let Some(children) = graph.get_children(&node) {
                    for child in children {
                        if let Some(deg) = degrees.get_mut(child) {
                            *deg = deg.saturating_sub(1);
                        }
                    }
                }
            }
        }

        Ok(core)
    }

    /// Compute depth of the k-core from root
    fn compute_depth(
        &self,
        kcore: &HashSet<MessageId>,
        root: MessageId,
        graph: &MessageGraph,
    ) -> u32 {
        let mut max_depth = 0;
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        
        queue.push_back((root, 0));
        visited.insert(root);

        while let Some((current, depth)) = queue.pop_front() {
            max_depth = max_depth.max(depth);
            
            if let Some(children) = graph.get_children(&current) {
                for child in children {
                    if kcore.contains(child) && visited.insert(*child) {
                        queue.push_back((*child, depth + 1));
                    }
                }
            }
        }

        max_depth
    }

    /// Compute reputation and diversity metrics
    fn compute_metrics(
        &self,
        kcore: &HashSet<MessageId>,
        graph: &MessageGraph,
    ) -> Result<(f64, HashMap<u32, usize>)> {
        let mut total_reputation = 0.0;
        let mut balls_per_axis: HashMap<u32, HashSet<Vec<u8>>> = HashMap::new();

        for &node in kcore {
            if let Some(info) = graph.get_message_info(&node) {
                total_reputation += info.reputation;
                
                for (axis_id, ball_id) in &info.ball_ids {
                    balls_per_axis
                        .entry(*axis_id)
                        .or_insert_with(HashSet::new)
                        .insert(ball_id.clone());
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

/// Simplified message graph interface
pub struct MessageGraph {
    parents: HashMap<MessageId, Vec<MessageId>>,
    children: HashMap<MessageId, Vec<MessageId>>,
    info: HashMap<MessageId, MessageInfo>,
}

#[derive(Debug, Clone)]
pub struct MessageInfo {
    pub reputation: f64,
    pub ball_ids: HashMap<u32, Vec<u8>>,
}

impl MessageGraph {
    pub fn new() -> Self {
        Self {
            parents: HashMap::new(),
            children: HashMap::new(),
            info: HashMap::new(),
        }
    }

    pub fn add_message(
        &mut self,
        id: MessageId,
        parents: Vec<MessageId>,
        info: MessageInfo,
    ) {
        self.parents.insert(id, parents.clone());
        
        for parent in parents {
            self.children
                .entry(parent)
                .or_insert_with(Vec::new)
                .push(id);
        }
        
        self.info.insert(id, info);
    }

    pub fn get_parents(&self, id: &MessageId) -> Option<&Vec<MessageId>> {
        self.parents.get(id)
    }

    pub fn get_children(&self, id: &MessageId) -> Option<&Vec<MessageId>> {
        self.children.get(id)
    }

    pub fn get_message_info(&self, id: &MessageId) -> Option<&MessageInfo> {
        self.info.get(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_graph() -> MessageGraph {
        let mut graph = MessageGraph::new();
        
        // Create a simple DAG structure
        let root = MessageId::new(b"root");
        let msg1 = MessageId::new(b"msg1");
        let msg2 = MessageId::new(b"msg2");
        let msg3 = MessageId::new(b"msg3");
        let msg4 = MessageId::new(b"msg4");
        
        graph.add_message(
            root,
            vec![],
            MessageInfo {
                reputation: 1.0,
                ball_ids: [(0, vec![0, 0]), (1, vec![1, 0])].into_iter().collect(),
            },
        );
        
        graph.add_message(
            msg1,
            vec![root],
            MessageInfo {
                reputation: 1.5,
                ball_ids: [(0, vec![0, 1]), (1, vec![1, 0])].into_iter().collect(),
            },
        );
        
        graph.add_message(
            msg2,
            vec![root],
            MessageInfo {
                reputation: 1.2,
                ball_ids: [(0, vec![0, 0]), (1, vec![1, 1])].into_iter().collect(),
            },
        );
        
        graph.add_message(
            msg3,
            vec![msg1, msg2],
            MessageInfo {
                reputation: 2.0,
                ball_ids: [(0, vec![0, 1]), (1, vec![1, 1])].into_iter().collect(),
            },
        );
        
        graph.add_message(
            msg4,
            vec![msg3],
            MessageInfo {
                reputation: 1.8,
                ball_ids: [(0, vec![0, 2]), (1, vec![1, 2])].into_iter().collect(),
            },
        );
        
        graph
    }

    #[test]
    fn test_kcore_analysis() {
        let graph = create_test_graph();
        let analyzer = KCoreAnalyzer::new(2, 2, 2, 3.0);
        
        let root = MessageId::new(b"root");
        let result = analyzer.analyze(root, &graph).unwrap();
        
        assert!(result.core_size > 0);
        // depth is u32, so it's always >= 0
    }

    #[test]
    fn test_forward_cone() {
        let graph = create_test_graph();
        let analyzer = KCoreAnalyzer::new(2, 2, 2, 3.0);
        
        let root = MessageId::new(b"root");
        let cone = analyzer.build_forward_cone(root, &graph).unwrap();
        
        assert!(cone.contains(&root));
        assert_eq!(cone.len(), 5); // root + 4 descendants
    }
}