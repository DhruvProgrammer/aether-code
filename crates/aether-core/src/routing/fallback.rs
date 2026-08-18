//! Fallback chain — when a model call fails, try the next model in the chain.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Why a fallback was triggered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackReason {
    /// Model returned a transient error (5xx, timeout, rate-limit).
    TransientError,
    /// Model returned a permanent error (4xx, bad request).
    PermanentError,
    /// Model failed the capability check (e.g. required tool not supported).
    CapabilityMismatch,
    /// Model failed health checks (recent failure rate too high).
    Unhealthy,
}

/// Outcome of a fallback chain execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackOutcome {
    pub succeeded: bool,
    pub attempts: usize,
    pub final_reason: Option<FallbackReason>,
}

/// An ordered list of model ids. Tries the first; on failure moves to the next.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackChain {
    pub models: Vec<String>,
}

impl FallbackChain {
    pub fn new(models: Vec<String>) -> Self {
        Self { models }
    }

    pub fn len(&self) -> usize {
        self.models.len()
    }

    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
    }

    /// Run `op` against each model in order. `op` returns `Err(reason)` on
    /// failure. Stops at the first success.
    pub async fn run<F, Fut, T>(
        &self,
        mut op: F,
    ) -> FallbackOutcome
    where
        F: FnMut(String) -> Fut,
        Fut: std::future::Future<Output = Result<T, FallbackReason>>,
    {
        let mut attempts = 0;
        for m in &self.models {
            attempts += 1;
            match op(m.clone()).await {
                Ok(_) => {
                    return FallbackOutcome {
                        succeeded: true,
                        attempts,
                        final_reason: None,
                    };
                }
                Err(_reason) => {
                    // Continue down the chain.
                }
            }
        }
        FallbackOutcome {
            succeeded: false,
            attempts,
            final_reason: Some(FallbackReason::TransientError),
        }
    }
}

pub type SharedFallbackChain = Arc<FallbackChain>;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn first_success_short_circuits() {
        let chain = FallbackChain::new(vec!["a".into(), "b".into(), "c".into()]);
        let outcome = chain
            .run(|_m| async { Ok::<_, FallbackReason>(()) })
            .await;
        assert!(outcome.succeeded);
        assert_eq!(outcome.attempts, 1);
    }

    #[tokio::test]
    async fn chains_through_failures() {
        let chain = FallbackChain::new(vec!["a".into(), "b".into(), "c".into()]);
        let outcome = chain
            .run(|m: String| async move {
                if m == "c" {
                    Ok(())
                } else {
                    Err(FallbackReason::TransientError)
                }
            })
            .await;
        assert!(outcome.succeeded);
        assert_eq!(outcome.attempts, 3);
    }

    #[tokio::test]
    async fn empty_chain_fails() {
        let chain = FallbackChain::new(vec![]);
        let outcome = chain.run(|_: String| async { Ok(()) }).await;
        assert!(!outcome.succeeded);
        assert_eq!(outcome.attempts, 0);
    }
}
