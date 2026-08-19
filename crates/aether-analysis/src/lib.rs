//! `aether-analysis` — pluggable code-analysis capability layer.
//!
//! SonarQube (and later ESLint / Semgrep / any analyzer) is exposed as a
//! **code analysis capability**, not an LLM. Deterministic static analysis
//! produces normalised [`finding::Finding`] records; AETHER's controller
//! (Model 2) reasons over them, and the executor (Model 1) implements fixes.
//!
//! Architecture (fixed authority chain):
//!
//! ```text
//! User
//!  ↓
//! Model 2 — Controller (decides WHEN analysis is useful, prioritises findings)
//!  ↓
//! Skills / Tools / Analysis  ← this crate lives here
//!  ↓
//! Model 1 — Executor (implements fixes)
//!  ↓
//! Codebase → Verification → Model 2
//! ```
//!
//! * [`AnalysisProvider`] is the plugin seam: `SonarQubeProvider` implements it
//!   today; ESLint/Semgrep providers plug in later without controller changes.
//! * Findings are **advisory**. Nothing in this crate executes commands from
//!   analyzer output or mutates files.
//! * Secrets are never surfaced: [`sanitize`] strips credentials before
//!   findings reach prompts, logs, the UI or checkpoints.

pub mod finding;
pub mod provider;
pub mod sonarqube;
pub mod compare;
pub mod store;
pub mod sanitize;
pub mod report;

pub use finding::{Finding, FindingKind, Location, Severity};
pub use provider::{AnalysisError, AnalysisProvider, AnalysisRequest, Availability};
pub use sonarqube::{SonarQubeConfig, SonarQubeMode, SonarQubeProvider};
pub use compare::{AnalysisDiff, diff};
pub use store::{AnalysisStore, StoreError};
pub use sanitize::sanitize_text;
pub use report::{AnalysisReport, SeverityDistribution, project_key};
