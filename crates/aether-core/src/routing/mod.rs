//! Intelligent model routing for AETHER.
//!
//! Replaces the simple keyword-based `select_model` heuristic with a proper
//! capability-aware router that picks the best model for a task based on:
//!
//! - Task type (code / review / research / plan / summarize / security / visual)
//! - Required capabilities (vision, reasoning, tools, json-mode, structured output)
//! - Context size vs model context window
//! - Latency tier preference (cheap / balanced / capable)
//! - Cost preference
//! - Provider health (live health check results from `aether-registry`)
//! - Rolling failure rate
//! - Fallback chain (try A, on failure fall back to B, etc.)
//!
//! OpenCode has only basic keyword routing; this is a substantial improvement.

pub mod task;
pub mod capability;
pub mod profile;
pub mod router;
pub mod fallback;
pub mod health;

pub use capability::{Capability, CapabilityMatrix, ModelCapabilities};
pub use fallback::{FallbackChain, FallbackOutcome, FallbackReason};
pub use health::{HealthScore, ModelHealth};
pub use profile::{LatencyTier, ModelProfile, RoutingHints};
pub use router::{RoutingDecision, RoutingReason, Router, RouterConfig};
pub use task::{TaskKind, TaskSignals};

