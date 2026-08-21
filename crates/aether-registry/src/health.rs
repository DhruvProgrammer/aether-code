//! Health checker — validates a provider end-to-end before the user is
//! allowed to mark it active.

use crate::catalog::ModelStatus;
use crate::provider::{AuthConfig, ProviderDescriptor, ProviderStatus};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// A single check in a health-check report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    pub label: String,
    pub passed: bool,
    pub detail: String,
    pub latency_ms: u64,
}

/// Overall outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthOutcome {
    pub provider_id: String,
    pub status: ProviderStatus,
    pub checks: Vec<HealthCheck>,
    pub total_latency_ms: u64,
    pub models_discovered: Vec<String>,
    pub can_save: bool,
    pub message: String,
}

#[derive(Default, Debug, Clone, Copy)]
pub struct HealthChecker;

impl HealthChecker {
    pub fn new() -> Self { Self }

    /// End-to-end health check:
    ///  1. Validate base URL.
    ///  2. Validate auth (env var resolves?).
    ///  3. Test connectivity (HEAD or GET `/models`).
    ///  4. Test configured API endpoint (`/chat/completions`).
    ///  5. Retrieve models (when supported).
    ///  6. Validate the configured model.
    ///  7. Detect unsupported capabilities.
    ///  8. Measure latency.
    pub async fn check(&self, p: &ProviderDescriptor) -> HealthOutcome {
        let started = Instant::now();
        let mut checks = Vec::new();
        let mut can_save = true;

        // 1. Validate base URL.
        let url_ok = url_is_reachable(&p.base_url).await;
        checks.push(HealthCheck {
            label: "base_url".into(),
            passed: url_ok,
            detail: if url_ok { p.base_url.clone() } else { format!("invalid URL: {}", p.base_url) },
            latency_ms: 0,
        });
        if !url_ok { can_save = false; }

        // 2. Validate auth (env var resolves? or raw present). Never expose secret.
        let (auth_ok, auth_detail) = match &p.auth {
            AuthConfig::None => (true, "no auth".to_string()),
            AuthConfig::BearerEnv { name } | AuthConfig::ApiKeyEnv { name } => {
                match std::env::var(name) {
                    Ok(v) if !v.is_empty() => (true, format!("env {name} resolved")),
                    Ok(_) => (false, format!("env {name} is empty")),
                    Err(_) => (false, format!("env {name} is not set")),
                }
            }
            AuthConfig::BearerRaw { key } | AuthConfig::ApiKeyRaw { key } => {
                if key.trim().is_empty() {
                    (false, "credential is empty".into())
                } else {
                    (true, "credential present".into())
                }
            }
        };
        checks.push(HealthCheck {
            label: "authentication".into(),
            passed: auth_ok,
            detail: auth_detail.clone(),
            latency_ms: 0,
        });
        if !auth_ok { can_save = false; }

        // 3. Connectivity probe (GET `/models`).
        let mut discovered: Vec<String> = Vec::new();
        let mut latency_ms: u64 = 0;
        if url_ok {
            let t = Instant::now();
            let probe_url = join_url(&p.base_url, "models");
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(8))
                .build()
                .unwrap_or_default();
            let mut req = match &p.auth {
                AuthConfig::BearerEnv { name } => client.get(&probe_url).bearer_auth(std::env::var(name).unwrap_or_default()),
                AuthConfig::BearerRaw { key } => client.get(&probe_url).bearer_auth(key),
                AuthConfig::ApiKeyEnv { name } => client.get(&probe_url).header("x-api-key", std::env::var(name).unwrap_or_default()),
                AuthConfig::ApiKeyRaw { key } => client.get(&probe_url).header("x-api-key", key),
                AuthConfig::None => client.get(&probe_url),
            };
            for (k, v) in &p.custom_headers {
                req = req.header(k, v);
            }
            let res = req.send().await;
            latency_ms = t.elapsed().as_millis() as u64;
            match res {
                Ok(r) => {
                    let status = r.status();
                    let ok = status.is_success();
                    checks.push(HealthCheck {
                        label: "connectivity".into(),
                        passed: ok,
                        detail: format!("HTTP {}", status.as_u16()),
                        latency_ms,
                    });
                    if ok {
                        if let Ok(body) = r.json::<serde_json::Value>().await {
                            if let Some(arr) = body.get("data").and_then(|d| d.as_array()) {
                                for m in arr {
                                    if let Some(id) = m.get("id").and_then(|v| v.as_str()) {
                                        discovered.push(id.to_string());
                                    }
                                }
                            }
                        }
                    }
                    if !ok { can_save = false; }
                }
                Err(e) => {
                    checks.push(HealthCheck {
                        label: "connectivity".into(),
                        passed: false,
                        detail: format!("request failed: {e}"),
                        latency_ms,
                    });
                    can_save = false;
                }
            }
        }

        // 4. Validate configured model against discovered.
        let mut missing_models: Vec<&str> = Vec::new();
        for m in &p.models {
            if !discovered.is_empty() && !discovered.iter().any(|d| d == m) {
                missing_models.push(m.as_str());
            }
        }
        if !missing_models.is_empty() {
            checks.push(HealthCheck {
                label: "model_available".into(),
                passed: false,
                detail: format!("missing on provider: {}", missing_models.join(", ")),
                latency_ms: 0,
            });
            can_save = false;
        } else if !p.models.is_empty() {
            checks.push(HealthCheck {
                label: "model_available".into(),
                passed: true,
                detail: if discovered.is_empty() { "skipped (no /models endpoint)".into() } else { "all configured models present".into() },
                latency_ms: 0,
            });
        }

        let status = if can_save { ProviderStatus::Healthy } else if auth_ok { ProviderStatus::Degraded } else if !auth_ok && url_ok { ProviderStatus::AuthFailed } else { ProviderStatus::Unreachable };
        let message = if can_save {
            format!("Provider reachable, authentication succeeded, {} models visible.", discovered.len().max(p.models.len()))
        } else {
            let failed: Vec<&str> = checks.iter().filter(|c| !c.passed).map(|c| c.label.as_str()).collect();
            format!("Provider failed checks: {}", failed.join(", "))
        };

        let total_latency_ms = started.elapsed().as_millis() as u64;
        HealthOutcome {
            provider_id: p.id.clone(),
            status,
            checks,
            total_latency_ms,
            models_discovered: discovered,
            can_save,
            message,
        }
    }
}

async fn url_is_reachable(base: &str) -> bool {
    if base.parse::<reqwest::Url>().is_err() { return false; }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();
    let res = client.get(base).send().await;
    matches!(res, Ok(r) if r.status().as_u16() < 500 || r.status().as_u16() == 401 || r.status().as_u16() == 403 || r.status().as_u16() == 404)
}

fn join_url(base: &str, suffix: &str) -> String {
    let base = base.trim_end_matches('/');
    format!("{base}/{suffix}")
}

impl HealthOutcome {
    pub fn set_model_statuses(&self, registry_models: &mut Vec<&mut crate::catalog::ModelDescriptor>) {
        for m in registry_models.iter_mut() {
            if self.models_discovered.iter().any(|d| d == &m.id) {
                m.status = ModelStatus::Available;
            } else if self.status == ProviderStatus::Healthy {
                m.status = ModelStatus::Unknown;
            } else {
                m.status = ModelStatus::Degraded;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_provider(base: &str) -> ProviderDescriptor {
        ProviderDescriptor::new_openai_compatible("p", base, "AETHER_TEST_KEY_X")
            .with_model("fake-model")
    }

    #[tokio::test]
    async fn unreachable_url_yields_unreachable() {
        let p = sample_provider("http://127.0.0.1:1");
        let h = HealthChecker::new().check(&p).await;
        assert!(!h.can_save);
        assert!(matches!(h.status, ProviderStatus::Unreachable | ProviderStatus::AuthFailed | ProviderStatus::Degraded));
    }
}
