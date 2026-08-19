//! Provider-neutral analysis abstraction.
//!
//! `AnalysisProvider` is the plugin seam: the controller interacts with this
//! trait, never with SonarQube specifics. New analyzers (ESLint, Semgrep, …)
//! implement the same trait and become drop-in capabilities.

use async_trait::async_trait;

use crate::finding::Finding;
use crate::report::AnalysisReport;

use serde::{Deserialize, Serialize};

/// What the controller asks an analyzer to do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisRequest {
    /// Absolute path of the project root to analyse.
    pub project_root: String,
    /// Optional stable project key (defaults to derived from root path).
    pub project_key: Option<String>,
    /// Human-readable label for this analysis run (e.g. "post-fix verification").
    pub label: Option<String>,
    /// Restrict analysis to these relative paths when supported.
    pub scope: Vec<String>,
}

impl AnalysisRequest {
    pub fn new(project_root: impl Into<String>) -> Self {
        Self {
            project_root: project_root.into(),
            project_key: None,
            label: None,
            scope: Vec::new(),
        }
    }
}

/// Whether a provider is available in this environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Availability {
    pub provider: String,
    pub available: bool,
    /// Why not available, or a summary of detected requirements.
    pub detail: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AnalysisError {
    #[error("provider not available: {0}")]
    Unavailable(String),
    #[error("analysis failed: {0}")]
    Failed(String),
    #[error("timed out after {0}s")]
    Timeout(u64),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

/// One code-analysis backend. Implementors must be safe to call concurrently
/// and must NEVER execute code derived from their own findings; they only run
/// their own analyzer binary / API.
#[async_trait]
pub trait AnalysisProvider: Send + Sync {
    /// Unique id, e.g. `"sonarqube"`.
    fn id(&self) -> &str;

    /// Human-readable display name, e.g. `"SonarQube"`.
    fn display_name(&self) -> &str;

    /// Probe whether this provider can run in the current environment.
    /// Must be cheap and side-effect free.
    async fn availability(&self) -> Availability;

    /// Run an analysis and return a normalised report.
    async fn analyze(&self, req: &AnalysisRequest) -> Result<AnalysisReport, AnalysisError>;

    /// Fetch the latest findings without re-running analysis, when the provider
    /// keeps server-side state (SonarQube does). Returns `None` when the
    /// provider has no stored results yet.
    async fn latest_findings(&self, req: &AnalysisRequest) -> Result<Option<Vec<Finding>>, AnalysisError> {
        let _ = req;
        Ok(None)
    }
}
