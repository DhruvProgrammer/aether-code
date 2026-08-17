//! Centralised AETHER Permission Engine.
//!
//! Replaces the v0.11 flat six-knob table with a full, hierarchical engine:
//!
//!   Global Policy
//!       ↓
//!   Project Policy
//!       ↓
//!   Agent-Role Policy
//!       ↓
//!   Individual Agent Policy
//!       ↓
//!   Tool Policy
//!       ↓
//!   Specific Operation
//!
//! Every decision is auditable (logged to a `DecisionLog`), emits a
//! `PermissionDecision` event, and can be approved interactively through the
//! `ApprovalRequest` / `ApprovalResponse` channel.
//!
//! Resolution granularity:
//!   * Agent id
//!   * Agent role
//!   * Tool name
//!   * Command (with pattern match)
//!   * File (absolute / glob / directory)
//!   * Operation kind (read / write / create / delete / execute / network / install / admin)
//!   * Provider id
//!   * Model id
//!   * Network host
//!   * Shell access (any / deny)
//!   * Process spawn
//!   * Git operation
//!   * Package installation
//!   * Environment variable access
//!   * Secret access
//!   * MCP server / integration
//!   * External API

pub mod policy;
pub mod rule;
pub mod scope;
pub mod engine;
pub mod decision;
pub mod approval;
pub mod event;
mod glob;

pub use policy::{Policy, Permission};
pub use rule::{Rule, RuleMatch, RuleSource};
pub use scope::{Operation, ResourceScope};
pub use engine::{PermissionEngine, BashLevel, classify_bash, classify_bash_with_label, is_dangerous, DecisionContext};
pub use decision::{DecisionLog, DecisionRecord, DecisionVerdict};
pub use approval::{ApprovalChannel, ApprovalRequest, ApprovalResponse, ApprovalScope};
pub use event::{PermissionEvent, PermissionEventSink};
