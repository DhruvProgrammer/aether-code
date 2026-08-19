//! Live API validation (gateway spec §10, §11, §12).
//!
//! Validation performs a **real, minimal** request against the configured
//! endpoint — reachability, auth, model access, and a valid response. It does
//! not trust populated fields alone. On success it returns the configuration
//! fingerprint (§11) and a persisted snapshot so the UI can gate Save/Activate.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::FailureClass;
use crate::fingerprint::{fingerprint_binding, ModelRoleSnapshot};
use crate::role::Role;

/// Everything needed to validate one role's binding. Contains only the
/// env-var *name* for the key, never the key value.
#[derive(Debug, Clone)]
pub struct ValidateTarget {
    pub role: Role,
    pub model_key: String,
    pub provider_id: String,
    pub base_url: String,
    pub model_id: String,
    pub api_key_env: String,
    pub extra_body: Option<serde_json::Value>,
}

/// Result of [`validate_binding`]. `ok` gates Save/Activate in the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationOutcome {
    pub ok: bool,
    /// Classification when not ok; None when ok.
    pub class: Option<FailureClass>,
    pub detail: String,
    pub latency_ms: u64,
    /// Present only when `ok`.
    pub fingerprint: Option<String>,
    pub snapshot: Option<ModelRoleSnapshot>,
}

impl ValidationOutcome {
    fn fail(class: FailureClass, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self {
            ok: false,
            class: Some(class),
            detail: format!("{} — {}", class.hint(), detail),
            latency_ms: 0,
            fingerprint: None,
            snapshot: None,
        }
    }
}

/// Pure classification of a probe HTTP (status, body) into a failure class.
/// Extracted so it can be unit-tested without hitting a network.
pub fn classify_probe(status: u16, body: &str) -> Result<(), FailureClass> {
    if (200..300).contains(&status) {
        return Ok(());
    }
    let class = FailureClass::from_http(status, body);
    // Never echo the request body (which could contain the key) — surface
    // only a bounded summary of the provider's own response.
    Err(class)
}

/// Run a real minimal chat-completions request against `target`.
///
/// * Uses an env-resolved key from `target.api_key_env`.
/// * Connects with a 10 s timeout so the settings UI never freezes (§20).
/// * Sends `max_tokens=1`, `stream=false`.
/// * On 2xx, computes the fingerprint and builds a snapshot.
pub async fn validate_binding(target: &ValidateTarget) -> ValidationOutcome {
    let started = std::time::Instant::now();

    let api_key = match std::env::var(&target.api_key_env) {
        Ok(v) if !v.is_empty() => v,
        _ => {
            return ValidationOutcome::fail(
                FailureClass::InvalidApiKey,
                format!("environment variable '{}' is not set", target.api_key_env),
            );
        }
    };

    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => return ValidationOutcome::fail(FailureClass::NetworkFailure, e.to_string()),
    };

    let mut body = serde_json::json!({
        "model": target.model_id,
        "messages": [{ "role": "user", "content": "ping" }],
        "max_tokens": 1,
        "stream": false,
    });
    if let Some(eb) = &target.extra_body {
        if let (Some(b), Some(o)) = (body.as_object_mut(), eb.as_object()) {
            for (k, v) in o {
                b.insert(k.clone(), v.clone());
            }
        }
    }

    let url = format!("{}/chat/completions", target.base_url.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .bearer_auth(&api_key)
        .json(&body)
        .send()
        .await;

    let latency_ms = started.elapsed().as_millis() as u64;
    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            let class = if e.is_timeout() {
                FailureClass::RequestTimeout
            } else if e.is_connect() {
                if e.is_request() {
                    FailureClass::InvalidBaseUrl
                } else {
                    FailureClass::EndpointUnavailable
                }
            } else {
                FailureClass::NetworkFailure
            };
            return ValidationOutcome::fail(class, e.to_string());
        }
    };

    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if let Err(class) = classify_probe(status, &text) {
        let mut out = ValidationOutcome::fail(class, bounded(&text));
        out.latency_ms = latency_ms;
        return out;
    }

    // Success: compute fingerprint + snapshot.
    let fingerprint = fingerprint_binding(
        target.role,
        &target.provider_id,
        &target.base_url,
        &target.model_id,
        &target.api_key_env,
        target.extra_body.as_ref(),
    );
    let snapshot = ModelRoleSnapshot {
        role: target.role,
        model_key: target.model_key.clone(),
        provider_id: target.provider_id.clone(),
        base_url: target.base_url.clone(),
        model_id: target.model_id.clone(),
        api_key_env: target.api_key_env.clone(),
        fingerprint: fingerprint.clone(),
        validated_at: chrono_now(),
    };
    ValidationOutcome {
        ok: true,
        class: None,
        detail: format!("validated {} @ {} model={}", target.provider_id, target.base_url, target.model_id),
        latency_ms,
        fingerprint: Some(fingerprint),
        snapshot: Some(snapshot),
    }
}

fn chrono_now() -> String {
    // Avoid pulling chrono into the gateway; RFC3339 via SystemTime is enough
    // for display + store ordering.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("ts:{}", now.as_secs())
}

fn bounded(s: &str) -> String {
    s.chars().take(300).collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_probe_ok_and_errors() {
        assert!(classify_probe(200, "{}").is_ok());
        assert_eq!(classify_probe(401, "bad").unwrap_err(), FailureClass::InvalidApiKey);
        assert_eq!(classify_probe(429, "slow down").unwrap_err(), FailureClass::RateLimited);
        assert_eq!(classify_probe(500, "oops").unwrap_err(), FailureClass::ServerError);
    }

    #[tokio::test]
    async fn validation_fails_when_key_env_missing() {
        std::env::remove_var("GW_TEST_MISSING_KEY");
        let t = ValidateTarget {
            role: Role::Executor,
            model_key: "m".into(),
            provider_id: "openai_compatible".into(),
            base_url: "https://example.invalid/v1".into(),
            model_id: "gpt".into(),
            api_key_env: "GW_TEST_MISSING_KEY".into(),
            extra_body: None,
        };
        let out = validate_binding(&t).await;
        assert!(!out.ok);
        assert_eq!(out.class, Some(FailureClass::InvalidApiKey));
        assert!(out.snapshot.is_none());
    }

    #[test]
    fn outcome_fail_carries_hint() {
        let o = ValidationOutcome::fail(FailureClass::RateLimited, "too many");
        assert!(!o.ok);
        assert!(o.detail.contains("rate-limited"));
    }
}
