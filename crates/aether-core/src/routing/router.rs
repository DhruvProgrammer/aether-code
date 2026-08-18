//! Intelligent model router â€” picks the best model for a given task.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::fallback::FallbackChain;
use super::health::ModelHealth;
use super::profile::{LatencyTier, ModelProfile, RoutingHints};
use super::task::{TaskSignals, TaskKind};

/// Why the router chose a particular model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingReason {
    pub signals: TaskSignals,
    pub preferred_tier: LatencyTier,
    /// All considered models with their scores, sorted descending.
    pub scored: Vec<(String, f32)>,
    /// Which capability / health gate disqualified candidates.
    pub disqualifications: Vec<String>,
}

/// A routing decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    pub model_id: String,
    pub reason: RoutingReason,
    pub fallback_chain: FallbackChain,
}

/// Router configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfig {
    /// Default preferred tier when signals don't override.
    pub default_tier: LatencyTier,
    /// Tier override per task kind.
    pub tier_overrides: std::collections::HashMap<String, LatencyTier>,
}

impl Default for RouterConfig {
    fn default() -> Self {
        let mut tier_overrides = std::collections::HashMap::new();
        tier_overrides.insert("summarize".into(), LatencyTier::Cheap);
        tier_overrides.insert("research".into(), LatencyTier::Balanced);
        tier_overrides.insert("review".into(), LatencyTier::Balanced);
        tier_overrides.insert("plan".into(), LatencyTier::Capable);
        tier_overrides.insert("code".into(), LatencyTier::Capable);
        tier_overrides.insert("security".into(), LatencyTier::Capable);
        tier_overrides.insert("test".into(), LatencyTier::Balanced);
        tier_overrides.insert("visual".into(), LatencyTier::Capable);
        Self {
            default_tier: LatencyTier::Balanced,
            tier_overrides,
        }
    }
}

/// The router itself.
pub struct Router {
    profiles: Vec<ModelProfile>,
    health: Vec<Arc<ModelHealth>>,
    config: RouterConfig,
}

impl Router {
    pub fn new(profiles: Vec<ModelProfile>, config: RouterConfig) -> Self {
        let health = profiles
            .iter()
            .map(|p| ModelHealth::new(p.id.clone()))
            .collect();
        Self {
            profiles,
            health,
            config,
        }
    }

    pub fn with_default_health(self) -> Self {
        self
    }

    pub fn health_for(&self, model_id: &str) -> Option<Arc<ModelHealth>> {
        self.health
            .iter()
            .find(|h| h.model_id() == model_id)
            .cloned()
    }

    pub fn record_outcome(&self, model_id: &str, success: bool, latency_ms: u32) {
        if let Some(h) = self.health_for(model_id) {
            h.record_outcome(success, latency_ms);
        }
    }

    pub fn profiles(&self) -> &[ModelProfile] {
        &self.profiles
    }

    pub fn config(&self) -> &RouterConfig {
        &self.config
    }

    /// Decide which model to use for a task.
    pub fn route(&self, signals: &TaskSignals, hints: &RoutingHints) -> Option<RoutingDecision> {
        if let Some(force) = &hints.force_model {
            if let Some(p) = self.profiles.iter().find(|p| &p.id == force) {
                let fallback = self.build_fallback_chain(p, hints, signals);
                return Some(RoutingDecision {
                    model_id: force.clone(),
                    reason: RoutingReason {
                        signals: signals.clone(),
                        preferred_tier: p.tier,
                        scored: vec![(force.clone(), 1.0)],
                        disqualifications: vec![],
                    },
                    fallback_chain: fallback,
                });
            }
        }
        let preferred_tier = hints
            .prefer_tier
            .unwrap_or_else(|| self.tier_for_kind(signals.kind));
        let mut disqualifications = Vec::new();
        let mut scored: Vec<(String, f32, &ModelProfile)> = self
            .profiles
            .iter()
            .filter(|p| !hints.forbid_models.contains(&p.id))
            .filter(|p| {
                if !p.capabilities.satisfies(&signals.required_capabilities) {
                    disqualifications.push(format!(
                        "{}: missing required capabilities",
                        p.id
                    ));
                    return false;
                }
                if !p.capabilities.fits_context(signals.estimated_context_tokens) {
                    disqualifications.push(format!(
                        "{}: context window too small (need {}, have {})",
                        p.id, signals.estimated_context_tokens, p.capabilities.context_window
                    ));
                    return false;
                }
                true
            })
            .map(|p| {
                let health = self.health_for(&p.id).map(|h| h.snapshot());
                let s = p.score(signals, preferred_tier, health.as_ref());
                (p.id.clone(), s, p)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let chosen = scored.first()?;
        let fallback = self.build_fallback_chain(chosen.2, hints, signals);
        Some(RoutingDecision {
            model_id: chosen.0.clone(),
            reason: RoutingReason {
                signals: signals.clone(),
                preferred_tier,
                scored: scored
                    .iter()
                    .map(|(id, s, _)| (id.clone(), *s))
                    .collect(),
                disqualifications,
            },
            fallback_chain: fallback,
        })
    }

    fn tier_for_kind(&self, kind: TaskKind) -> LatencyTier {
        let key = match kind {
            TaskKind::Code => "code",
            TaskKind::Review => "review",
            TaskKind::Research => "research",
            TaskKind::Plan => "plan",
            TaskKind::Summarize => "summarize",
            TaskKind::Security => "security",
            TaskKind::Test => "test",
            TaskKind::Visual => "visual",
            TaskKind::Generic => "generic",
        };
        self.config
            .tier_overrides
            .get(key)
            .copied()
            .unwrap_or(self.config.default_tier)
    }

    fn build_fallback_chain(
        &self,
        primary: &ModelProfile,
        hints: &RoutingHints,
        signals: &TaskSignals,
    ) -> FallbackChain {
        // Fallback = remaining capable models in the same preferred tier,
        // excluding the primary and any forbidden.
        let mut models = vec![primary.id.clone()];
        for p in &self.profiles {
            if p.id == primary.id || hints.forbid_models.contains(&p.id) {
                continue;
            }
            if !p.capabilities.satisfies(&signals.required_capabilities) {
                continue;
            }
            if !p.capabilities.fits_context(signals.estimated_context_tokens) {
                continue;
            }
            models.push(p.id.clone());
        }
        FallbackChain::new(models)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::capability::{Capability, ModelCapabilities};

    fn profile(id: &str, tier: LatencyTier, window: u32, caps: Vec<Capability>) -> ModelProfile {
        let mc = caps
            .into_iter()
            .fold(ModelCapabilities::new().with_window(window), |m, c| m.with(c));
        ModelProfile::new(id, tier)
            .with_display_name(id)
            .with_capabilities(mc)
    }

    #[test]
    fn empty_router_returns_none() {
        let r = Router::new(vec![], RouterConfig::default());
        let s = TaskSignals::classify("implement foo", 0);
        assert!(r.route(&s, &RoutingHints::default()).is_none());
    }

    #[test]
    fn picks_highest_scoring_model() {
        let profiles = vec![
            profile("cheap", LatencyTier::Cheap, 8000, vec![Capability::ToolCalling, Capability::Streaming]),
            profile("capable", LatencyTier::Capable, 32_000, vec![Capability::ToolCalling, Capability::Streaming]),
        ];
        let r = Router::new(profiles, RouterConfig::default());
        let s = TaskSignals::classify("implement OAuth login with refresh tokens", 0);
        let d = r.route(&s, &RoutingHints::default()).unwrap();
        assert_eq!(d.model_id, "capable");
    }

    #[test]
    fn honors_force_model() {
        let profiles = vec![
            profile("cheap", LatencyTier::Cheap, 8000, vec![Capability::ToolCalling, Capability::Streaming]),
            profile("capable", LatencyTier::Capable, 32_000, vec![Capability::ToolCalling, Capability::Streaming]),
        ];
        let r = Router::new(profiles, RouterConfig::default());
        let s = TaskSignals::classify("implement OAuth", 0);
        let d = r
            .route(
                &s,
                &RoutingHints {
                    force_model: Some("cheap".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(d.model_id, "cheap");
    }

    #[test]
    fn honors_forbid() {
        let profiles = vec![
            profile("cheap", LatencyTier::Cheap, 8000, vec![Capability::ToolCalling, Capability::Streaming]),
            profile("capable", LatencyTier::Capable, 32_000, vec![Capability::ToolCalling, Capability::Streaming]),
        ];
        let r = Router::new(profiles, RouterConfig::default());
        let s = TaskSignals::classify("implement OAuth", 0);
        let d = r
            .route(
                &s,
                &RoutingHints {
                    forbid_models: vec!["capable".into()],
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(d.model_id, "cheap");
    }

    #[test]
    fn fallback_chain_excludes_primary_dups() {
        let profiles = vec![
            profile("a", LatencyTier::Capable, 8000, vec![Capability::ToolCalling, Capability::Streaming]),
            profile("b", LatencyTier::Capable, 8000, vec![Capability::ToolCalling, Capability::Streaming]),
            profile("c", LatencyTier::Capable, 8000, vec![Capability::ToolCalling, Capability::Streaming]),
        ];
        let r = Router::new(profiles, RouterConfig::default());
        let s = TaskSignals::classify("implement OAuth", 0);
        let d = r.route(&s, &RoutingHints::default()).unwrap();
        assert!(d.fallback_chain.len() >= 2);
    }

    #[test]
    fn health_failure_downweights() {
        let profiles = vec![profile("only", LatencyTier::Capable, 8000, vec![Capability::ToolCalling, Capability::Streaming])];
        let r = Router::new(profiles, RouterConfig::default());
        // Make the only model unhealthy
        for _ in 0..10 {
            r.record_outcome("only", false, 5000);
        }
        let s = TaskSignals::classify("implement OAuth", 0);
        let d = r.route(&s, &RoutingHints::default()).unwrap();
        assert_eq!(d.model_id, "only"); // still picked, but health factor should be < 1.0
        let score = d.reason.scored.first().unwrap().1;
        assert!(score < 1.5);
    }
}
