//! Agent orchestration core (spec §2, §3, §4, §29).
//! Controller = persistent orchestration/memory layer; Executor = swappable worker.

pub mod agent_loop;
pub mod agents;
pub mod controller;
pub mod eng;
pub mod executor;
pub mod mode;
pub mod prompt;
pub mod subagents;
pub mod visual;
