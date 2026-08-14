//! First-class multi-agent subsystem (spec: full_design.md, phases 2-3 + partial 5).
//!
//! Brings agent definitions, a registry, routing, a context builder, lifecycle tracking, and a
//! runner (`AgentTool` primitive) around AETHER's existing two-LLM Controller/Executor core.
//! The SMALL LLM (controller) orchestrates; the BIG LLM (executor) implements via the
//! `implementer` agent. Other agents run on the SMALL LLM.

pub mod context;
pub mod definition;
pub mod lifecycle;
pub mod registry;
pub mod router;
pub mod runner;

pub use context::build as build_agent_context;
pub use definition::{AgentBudget, AgentDefinition, AgentPermissions, parse_perm};
pub use lifecycle::{AgentRun, AgentStatus, LifecycleTracker};
pub use registry::AgentRegistry;
pub use router::AgentRouter;
pub use runner::{run_agent, run_agent_resolved};
