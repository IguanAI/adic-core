use adic_types::MessageId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectionStep {
    pub step_type: StepType,
    pub timestamp: i64,
    pub details: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StepType {
    AxisCandidates,
    Widening,
    Sampling,
    Validation,
    Success,
    Failure,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MrwTrace {
    pub steps: Vec<SelectionStep>,
    pub selected_parents: Vec<MessageId>,
    pub success: bool,
    pub total_candidates_considered: usize,
    pub widen_count: u32,
    pub final_horizon: usize,
}

impl Default for MrwTrace {
    fn default() -> Self {
        Self::new()
    }
}

impl MrwTrace {
    pub fn new() -> Self {
        Self {
            steps: Vec::new(),
            selected_parents: Vec::new(),
            success: false,
            total_candidates_considered: 0,
            widen_count: 0,
            final_horizon: 0,
        }
    }

    pub fn record_axis_candidates(&mut self, axis: u32, count: usize) {
        let mut details = HashMap::new();
        details.insert("axis".to_string(), axis.to_string());
        details.insert("candidates".to_string(), count.to_string());

        self.steps.push(SelectionStep {
            step_type: StepType::AxisCandidates,
            timestamp: chrono::Utc::now().timestamp_millis(),
            details,
        });

        self.total_candidates_considered += count;
    }

    pub fn record_widen(&mut self, new_horizon: usize) {
        let mut details = HashMap::new();
        details.insert("new_horizon".to_string(), new_horizon.to_string());
        details.insert(
            "widen_count".to_string(),
            (self.widen_count + 1).to_string(),
        );

        self.steps.push(SelectionStep {
            step_type: StepType::Widening,
            timestamp: chrono::Utc::now().timestamp_millis(),
            details,
        });

        self.widen_count += 1;
        self.final_horizon = new_horizon;
    }

    pub fn record_sampling(&mut self, sampled: usize, from_pool: usize) {
        let mut details = HashMap::new();
        details.insert("sampled".to_string(), sampled.to_string());
        details.insert("pool_size".to_string(), from_pool.to_string());

        self.steps.push(SelectionStep {
            step_type: StepType::Sampling,
            timestamp: chrono::Utc::now().timestamp_millis(),
            details,
        });
    }

    pub fn record_validation(&mut self, passed: bool, reason: String) {
        let mut details = HashMap::new();
        details.insert("passed".to_string(), passed.to_string());
        details.insert("reason".to_string(), reason);

        self.steps.push(SelectionStep {
            step_type: StepType::Validation,
            timestamp: chrono::Utc::now().timestamp_millis(),
            details,
        });
    }

    pub fn record_success(&mut self, parents: Vec<MessageId>) {
        let mut details = HashMap::new();
        details.insert("parent_count".to_string(), parents.len().to_string());

        self.steps.push(SelectionStep {
            step_type: StepType::Success,
            timestamp: chrono::Utc::now().timestamp_millis(),
            details,
        });

        self.selected_parents = parents;
        self.success = true;
    }

    pub fn record_failure(&mut self, reason: String) {
        let mut details = HashMap::new();
        details.insert("reason".to_string(), reason);

        self.steps.push(SelectionStep {
            step_type: StepType::Failure,
            timestamp: chrono::Utc::now().timestamp_millis(),
            details,
        });

        self.success = false;
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn duration_ms(&self) -> Option<i64> {
        if self.steps.len() < 2 {
            return None;
        }

        let first = self.steps.first().unwrap();
        let last = self.steps.last().unwrap();
        Some(last.timestamp - first.timestamp)
    }

    pub fn step_count(&self) -> usize {
        self.steps.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_recording() {
        let mut trace = MrwTrace::new();

        trace.record_axis_candidates(0, 10);
        trace.record_axis_candidates(1, 15);
        trace.record_axis_candidates(2, 12);

        assert_eq!(trace.total_candidates_considered, 37);
        assert_eq!(trace.step_count(), 3);

        trace.record_widen(20);
        assert_eq!(trace.widen_count, 1);
        assert_eq!(trace.final_horizon, 20);

        let parents = vec![
            MessageId::new(b"p1"),
            MessageId::new(b"p2"),
            MessageId::new(b"p3"),
            MessageId::new(b"p4"),
        ];
        trace.record_success(parents.clone());

        assert!(trace.success);
        assert_eq!(trace.selected_parents, parents);
    }

    #[test]
    fn test_trace_serialization() {
        let mut trace = MrwTrace::new();
        trace.record_axis_candidates(0, 5);
        trace.record_validation(true, "C2 diversity check passed".to_string());

        let json = trace.to_json().unwrap();
        assert!(json.contains("AxisCandidates"));
        assert!(json.contains("Validation"));

        let deserialized: MrwTrace = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.steps.len(), trace.steps.len());
    }
}
