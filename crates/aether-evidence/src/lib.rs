//! `aether-evidence` — structured evidence engine for AETHER.
//!
//! OpenCode has no notion of evidence; agents just emit prose ("I think this
//! works"). AETHER's 3-LLM hierarchical controller is built around the
//! principle that the controller cannot accept a subagent's output without
//! grounded evidence: what was changed, what was tested, what was verified,
//! what contradicts the claim, and how confident the source agent is.
//!
//! # Concepts
//!
//! * [`Evidence`] — a single structured claim made by an agent. Includes:
//!     - `claim` — the assertion (e.g. "implemented OAuth login")
//!     - `files` — files changed (relative paths)
//!     - `tool_results` — refs to specific tool invocations and their outputs
//!     - `tests` — test refs (path + name + result)
//!     - `confidence` — the agent's self-reported confidence, 0.0..=1.0
//!     - `contradictions` — explicit caveats or conflicting evidence
//!     - `recommendation` — pass / replan / manual-review
//! * [`EvidenceBag`] — collection of evidence records, indexed by source agent.
//! * [`Decision`] — what the controller concluded after aggregating evidence.
//!   Combines per-record confidence into a final verdict with reasoning.

pub mod evidence;
pub mod bag;
pub mod decision;

pub use evidence::{
    Confidence, Contradiction, Evidence, EvidenceId, EvidenceKind, Recommendation, TestRef, ToolResultRef,
};
pub use bag::EvidenceBag;
pub use decision::{decide, Decision, DecisionReasoning, Verdict};
