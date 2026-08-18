//! Routing profile — describes one available model.

use serde::{Deserialize, Serialize};

use super::capability::ModelCapabilities;

/// Latency tier preference. The router prefers cheaper / faster models for
/// low-complexity tasks and stronger models for high-complexity tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyTier {
    /// Fastest / cheapest. Prefer for trivial or summarisation tasks.
    Cheap,
    /// Balanced. Default.
    Balanced,
    /// Most capable. Use for complex architecture, security, deep review.
    Capable,
}

/// Hints the user can attach to a routing decision.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoutingHints {
    /// Force a specific latency tier.
    pub prefer_tier: Option<LatencyTier>,
    /// Force a specific model id (skip routing entirely).
    pub force_model: Option<String>,
    /// Force-fail this list of model ids (e.g. known-bad providers).
    pub forbid_models: Vec<String>,
    /// Extra capabilities the task absolutely requires.
    pub require_capabilities: Vec<super::capability::Capability>,
}

/// A model registered for routing — id + capabilities + cost hints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProfile {
    pub id: String,
    pub display_name: String,
    pub tier: LatencyTier,
    pub capabilities: ModelCapabilities,
    /// Approximate cost per 1k input tokens, in micro-cents. 0 = free.
    pub cost_input_per_1k: u32,
    /// Approximate cost per 1k output tokens, in micro-cents. 0 = free.
    pub cost_output_per_1k: u32,
}

impl ModelProfile {
    pub fn new(id: impl Into<String>, tier: LatencyTier) -> Self {
        Self {
            id: id.into(),
            display_name: String::new(),
            tier,
            capabilities: ModelCapabilities::new(),
            cost_input_per_1k: 0,
            cost_output_per_1k: 0,
        }
    }

    pub fn with_display_name(mut self, n: impl Into<String>) -> Self {
        self.display_name = n.into();
        self
    }

    pub fn with_capabilities(mut self, c: ModelCapabilities) -> Self {
        self.capabilities = c;
        self
    }

    pub fn with_cost(mut self, input: u32, output: u32) -> Self {
        self.cost_input_per_1k = input;
        self.cost_output_per_1k = output;
        self
    }

    /// Score this profile for a task with given signals & preferred tier.
    /// Higher score = better fit.
    pub fn score(
        &self,
        signals: &super::task::TaskSignals,
        preferred_tier: LatencyTier,
        health: Option<&super::health::HealthScore>,
    ) -> f32 {
        // Hard disqualifiers first.
        if !self.capabilities.satisfies(&signals.required_capabilities) {
            return f32::NEG_INFINITY;
        }
        if !self.capabilities.fits_context(signals.estimated_context_tokens) {
            return f32::NEG_INFINITY;
        }

        // Tier alignment: -1, 0, +1.
        let tier_diff = match (self.tier, preferred_tier) {
            (LatencyTier::Cheap, LatencyTier::Cheap) => 0.0,
            (LatencyTier::Cheap, LatencyTier::Balanced) => -0.2,
            (LatencyTier::Cheap, LatencyTier::Capable) => -0.5,
            (LatencyTier::Balanced, LatencyTier::Cheap) => 0.1,
            (LatencyTier::Balanced, LatencyTier::Balanced) => 0.0,
            (LatencyTier::Balanced, LatencyTier::Capable) => -0.3,
            (LatencyTier::Capable, LatencyTier::Cheap) => -0.5,
            (LatencyTier::Capable, LatencyTier::Balanced) => 0.2,
            (LatencyTier::Capable, LatencyTier::Capable) => 0.5,
        };
        // Complexity alignment: for high-complexity tasks, capable wins.
        let complexity_bonus = match self.tier {
            LatencyTier::Capable => signals.complexity * 0.5,
            LatencyTier::Balanced => (1.0 - signals.complexity) * 0.2,
            LatencyTier::Cheap => (1.0 - signals.complexity) * 0.4,
        };
        // Health multiplier (default 1.0; unknown health = neutral).
        let h = health.map(|h| h.multiplier()).unwrap_or(1.0);
        // Cost: penalise higher cost slightly (relative, very rough).
        let cost_penalty = (self.cost_input_per_1k as f32).log10().max(0.0) * 0.05;

        // Health is multiplicative: an unhealthy model (h < 1) scales the
        // whole score, so it can never outscore a healthy one at the same
        // capability tier.
        (1.0 + tier_diff + complexity_bonus - cost_penalty) * h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::capability::Capability;
    use crate::routing::task::{TaskKind, TaskSignals};

    fn signals(kind: TaskKind, complexity: f32, required: Vec<Capability>) -> TaskSignals {
        TaskSignals {
            kind,
            required_capabilities: required,
            complexity,
            estimated_context_tokens: 1000,
        }
    }

    #[test]
    fn scoring_prefers_matching_tier() {
        let caps = ModelCapabilities::new().with(Capability::ToolCalling);
        let cheap = ModelProfile::new("cheap", LatencyTier::Cheap).with_capabilities(caps.clone());
        let capable = ModelProfile::new("capable", LatencyTier::Capable).with_capabilities(caps);
        let s = signals(TaskKind::Code, 0.9, vec![Capability::ToolCalling]);
        assert!(
            capable.score(&s, LatencyTier::Capable, None)
                > cheap.score(&s, LatencyTier::Capable, None)
        );
    }

    #[test]
    fn missing_capability_disqualifies() {
        let m = ModelProfile::new("no-tools", LatencyTier::Capable);
        let s = signals(TaskKind::Code, 0.9, vec![Capability::ToolCalling]);
        assert_eq!(m.score(&s, LatencyTier::Capable, None), f32::NEG_INFINITY);
    }

    #[test]
    fn context_too_large_disqualifies() {
        let m = ModelProfile::new("small", LatencyTier::Capable)
            .with_capabilities(ModelCapabilities::new().with_window(100));
        let mut s = signals(TaskKind::Code, 0.5, vec![Capability::ToolCalling]);
        s.estimated_context_tokens = 10_000;
        assert_eq!(m.score(&s, LatencyTier::Capable, None), f32::NEG_INFINITY);
    }
}
