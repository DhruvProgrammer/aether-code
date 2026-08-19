//! The Model Gateway — the single model-access layer (spec sec 2, 3, 19, 20).
//!
//! Resolves each request to the **explicitly configured** provider+model for
//! the requesting role and dispatches it with timeout + cancellation.
//! Provider failures are isolated per-role (sec 15) and reported — never
//! hidden, never routed around.
//!
//! Concurrency: role lookup is a plain read on an immutable map, so a request
//! to one provider never blocks another (sec 19). The gateway owns no
//! routing, scoring, or fallback of any kind.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use aether_models::{ModelProvider, ProviderError};
use tokio::sync::Notify;

use crate::capability::{precheck, ModelCapabilities};
use crate::error::{FailureClass, GatewayError};
use crate::request::{Capability, GatewayRequest, GatewayResponse};
use crate::role::{Role, RoleBinding};

/// One provider adapter + the model id it serves for a role. Fixed once
/// installed for the lifetime of this gateway instance — hidden provider
/// switching is forbidden (sec 18).
#[derive(Clone)]
pub struct RoleProvider {
    pub binding: RoleBinding,
    pub model_key: String,
    pub provider_id: String,
    pub model_id: String,
    pub capabilities: ModelCapabilities,
    pub provider: Arc<dyn ModelProvider>,
}

/// Gateway configuration. All timeouts are explicit; nothing is infinite.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// Default total timeout per model request (bounded so the UI never
    /// freezes; sec 20). Overridable per-request via `GatewayRequest::timeout`.
    pub default_timeout: Duration,
    /// Timeout used for connectivity/validation probes.
    pub probe_timeout: Duration,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(180),
            probe_timeout: Duration::from_secs(30),
        }
    }
}

/// The Model Gateway. Cheap to clone — internals are `Arc`s so every
/// subsystem can hold a handle.
#[derive(Clone)]
pub struct ModelGateway {
    config: Arc<GatewayConfig>,
    /// role -> installed provider. Fixed at construction.
    roles: Arc<HashMap<Role, RoleProvider>>,
    /// Cancellation signal; checked per request via `tokio::select!`.
    cancel: Arc<Notify>,
}

impl ModelGateway {
    /// Build a gateway from resolved role bindings. Missing providers for
    /// optional roles are tolerated; missing providers for required roles
    /// surface later as `NotConfigured` from `provider_for`.
    pub fn new(
        config: GatewayConfig,
        bindings: &[RoleBinding],
        providers: &HashMap<String, Arc<dyn ModelProvider>>,
        model_ids: &HashMap<String, String>,
        provider_ids: &HashMap<String, String>,
        capabilities: &HashMap<String, ModelCapabilities>,
    ) -> Self {
        let mut roles: HashMap<Role, RoleProvider> = HashMap::new();
        for b in bindings {
            if !b.enabled {
                continue;
            }
            if let Some(p) = providers.get(&b.model_key) {
                roles.insert(
                    b.role,
                    RoleProvider {
                        binding: b.clone(),
                        model_key: b.model_key.clone(),
                        provider_id: provider_ids.get(&b.model_key).cloned().unwrap_or_default(),
                        model_id: model_ids
                            .get(&b.model_key)
                            .cloned()
                            .unwrap_or_else(|| b.model_key.clone()),
                        capabilities: capabilities.get(&b.model_key).cloned().unwrap_or_default(),
                        provider: p.clone(),
                    },
                );
            }
        }
        Self {
            config: Arc::new(config),
            roles: Arc::new(roles),
            cancel: Arc::new(Notify::new()),
        }
    }

    /// Trigger cancellation for all in-flight requests.
    pub fn cancel_all(&self) {
        self.cancel.notify_waiters();
    }

    pub fn cancel_handle(&self) -> Arc<Notify> {
        self.cancel.clone()
    }

    /// The provider configured for `role`, or a typed error. Never substitutes
    /// another role/provider (sec 8, 17).
    pub fn provider_for(&self, role: Role) -> Result<RoleProvider, GatewayError> {
        self.roles
            .get(&role)
            .cloned()
            .ok_or_else(|| GatewayError::NotConfigured(role.as_str().to_string()))
    }

    /// True when `role` is installed with a live provider.
    pub fn is_configured(&self, role: Role) -> bool {
        self.roles.contains_key(&role)
    }

    /// List installed roles (for settings display / observability metadata).
    pub fn installed(&self) -> Vec<(Role, String, String)> {
        let mut v: Vec<(Role, String, String)> = self
            .roles
            .iter()
            .map(|(r, p)| (*r, p.binding.model_key.clone(), p.provider_id.clone()))
            .collect();
        v.sort_by_key(|(r, _, _)| r.as_str());
        v
    }

    /// Dispatch a completion for the requesting role's configured provider.
    /// Capability pre-check -> timeout + cancellation -> classified errors.
    pub async fn complete(&self, req: GatewayRequest) -> Result<GatewayResponse, GatewayError> {
        let rp = self.provider_for(req.role)?;

        // Capability pre-check against the explicitly configured model. If
        // unsupported, fail with a capability error — never switch models.
        precheck(
            &rp.capabilities,
            req.required,
            req.role,
            req.tools.is_some(),
            req.images.as_ref().map(|i| !i.is_empty()).unwrap_or(false),
        )?;

        let timeout = req.timeout.unwrap_or(self.config.default_timeout);
        let cr = req.into_completion(rp.model_id.clone());
        let role = rp.binding.role;
        let model_key = rp.model_key.clone();
        let model_id = cr.model.clone();
        let provider_id = rp.provider_id.clone();
        let provider = rp.provider.clone();
        let started = std::time::Instant::now();

        let cancel = self.cancel.notified();
        tokio::pin!(cancel);

        let fut = async move { provider.complete(cr).await };
        let res = tokio::select! {
            r = fut => r,
            _ = &mut cancel => {
                return Err(GatewayError::Cancelled(format!("request for {} cancelled", role.as_str())));
            }
            _ = tokio::time::sleep(timeout) => {
                return Err(GatewayError::Provider {
                    source: None,
                    class: FailureClass::RequestTimeout,
                    detail: format!(
                        "{} request to {} timed out after {}s",
                        role.as_str(), provider_id, timeout.as_secs()
                    ),
                });
            }
        };

        match res {
            Ok(c) => {
                let mut out =
                    GatewayResponse::from_completion(role, model_key, model_id, provider_id, c);
                out.latency_ms = started.elapsed().as_millis() as u64;
                Ok(out)
            }
            Err(e) => Err(classify_provider_error(e)),
        }
    }

    /// Streaming variant. The stream is bound to the same provider; the outer
    /// completion call is cancelled on notify/timeout, and consumers of the
    /// stream should map stream errors through [`classify_provider_error`].
    pub async fn stream(
        &self,
        req: GatewayRequest,
    ) -> Result<(GatewayResponse, aether_models::TokenStream), GatewayError> {
        let rp = self.provider_for(req.role)?;
        if req.required == Capability::Vision
            && !rp.capabilities.vision
            && req.images.as_ref().map(|i| !i.is_empty()).unwrap_or(false)
        {
            return Err(GatewayError::CapabilityDenied {
                role: req.role.as_str().into(),
                class: FailureClass::UnsupportedCapability,
                detail: "configured model does not support vision".into(),
            });
        }
        if !rp.capabilities.streaming {
            return Err(GatewayError::CapabilityDenied {
                role: req.role.as_str().into(),
                class: FailureClass::UnsupportedCapability,
                detail: "configured model does not support streaming".into(),
            });
        }

        let timeout = req.timeout.unwrap_or(self.config.default_timeout);
        let cr = req.into_completion(rp.model_id.clone());
        let role = rp.binding.role;
        let model_key = rp.model_key.clone();
        let model_id = cr.model.clone();
        let provider_id = rp.provider_id.clone();
        let provider = rp.provider.clone();

        let cancel = self.cancel.notified();
        tokio::pin!(cancel);
        let fut = async move { provider.stream(cr).await };
        let res = tokio::select! {
            r = fut => r,
            _ = &mut cancel => {
                return Err(GatewayError::Cancelled(format!("stream for {} cancelled", role.as_str())));
            }
            _ = tokio::time::sleep(timeout) => {
                return Err(GatewayError::Provider {
                    source: None,
                    class: FailureClass::RequestTimeout,
                    detail: "stream connect timed out".into(),
                });
            }
        };
        match res {
            Ok(s) => {
                let head = GatewayResponse {
                    role,
                    model_key,
                    model_id,
                    provider_id,
                    content: None,
                    tool_calls: Vec::new(),
                    usage: None,
                    latency_ms: 0,
                };
                Ok((head, s))
            }
            Err(e) => Err(classify_provider_error(e)),
        }
    }

    /// Embeddings on the executor's configured provider (memory subsystem).
    /// Failure here is isolated — it must not break unrelated model roles
    /// (sec 15).
    pub async fn embeddings(
        &self,
        role: Role,
        input: Vec<String>,
    ) -> Result<Vec<Vec<f32>>, GatewayError> {
        let rp = self.provider_for(role)?;
        let timeout = self.config.default_timeout;
        let provider = rp.provider.clone();
        let fut = async move { provider.embeddings(input).await };
        let res = tokio::time::timeout(timeout, fut).await;
        match res {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(classify_provider_error(e)),
            Err(_) => Err(GatewayError::Provider {
                source: None,
                class: FailureClass::RequestTimeout,
                detail: "embedding request timed out".into(),
            }),
        }
    }
}

/// Map a low-level [`ProviderError`] onto a classified [`GatewayError`].
/// Provider errors are split on HTTP status where available so failures are
/// never shown as a bare "API Error" (sec 13).
pub fn classify_provider_error(e: ProviderError) -> GatewayError {
    match e {
        ProviderError::ApiStatus { status, body } => {
            let class = FailureClass::from_http(status, &body);
            GatewayError::Provider {
                source: None,
                class,
                detail: bounded(&body),
            }
        }
        ProviderError::Http(he) => {
            let class = if he.is_timeout() {
                FailureClass::RequestTimeout
            } else if he.is_connect() {
                if he.is_request() {
                    FailureClass::InvalidBaseUrl
                } else {
                    FailureClass::EndpointUnavailable
                }
            } else {
                FailureClass::NetworkFailure
            };
            GatewayError::Provider {
                source: Some(Box::new(he)),
                class,
                detail: "network-level provider failure".into(),
            }
        }
        ProviderError::Api(msg) => GatewayError::Provider {
            source: None,
            class: FailureClass::UnknownProvider,
            detail: bounded(&msg),
        },
        ProviderError::Json(je) => GatewayError::Provider {
            source: Some(Box::new(je)),
            class: FailureClass::InvalidRequest,
            detail: "provider returned invalid JSON".into(),
        },
        ProviderError::MissingEnv(name) => GatewayError::Provider {
            source: None,
            class: FailureClass::InvalidApiKey,
            detail: format!("environment variable '{name}' is not set"),
        },
    }
}

fn bounded(s: &str) -> String {
    s.chars().take(300).collect::<String>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aether_models::{CompletionRequest, CompletionResponse, ToolCall, Usage};
    use async_trait::async_trait;

    struct FakeProvider {
        fail_status: Option<u16>,
        delay: Duration,
    }

    #[async_trait]
    impl ModelProvider for FakeProvider {
        fn name(&self) -> &str { "fake" }
        fn supports_tool_calling(&self) -> bool { true }
        async fn complete(&self, _r: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
            if let Some(status) = self.fail_status {
                return Err(ProviderError::ApiStatus { status, body: "fake failure".into() });
            }
            tokio::time::sleep(self.delay).await;
            Ok(CompletionResponse {
                content: Some("ok".into()),
                tool_calls: vec![ToolCall { id: "1".into(), name: "t".into(), arguments: serde_json::json!({}) }],
                usage: Some(Usage { prompt_tokens: 1, completion_tokens: 2 }),
            })
        }
        async fn stream(&self, _r: CompletionRequest) -> Result<aether_models::TokenStream, ProviderError> {
            Err(ProviderError::Api("no stream in fake".into()))
        }
        async fn embeddings(&self, _i: Vec<String>) -> Result<Vec<Vec<f32>>, ProviderError> {
            Ok(vec![vec![0.1, 0.2]])
        }
    }

    fn gw(providers: HashMap<String, Arc<dyn ModelProvider>>, bindings: &[RoleBinding]) -> ModelGateway {
        let mut model_ids = HashMap::new();
        let mut provider_ids = HashMap::new();
        let caps = HashMap::new();
        for b in bindings {
            model_ids.insert(b.model_key.clone(), b.model_key.clone());
            provider_ids.insert(b.model_key.clone(), "fake".to_string());
        }
        ModelGateway::new(GatewayConfig::default(), bindings, &providers, &model_ids, &provider_ids, &caps)
    }

    #[tokio::test]
    async fn complete_resolves_configured_role() {
        let providers: HashMap<String, Arc<dyn ModelProvider>> = [(
            "nvidia-a".to_string(),
            Arc::new(FakeProvider { fail_status: None, delay: Duration::from_millis(1) }) as Arc<dyn ModelProvider>,
        )].into_iter().collect();
        let bindings = vec![RoleBinding::new(Role::Executor, "nvidia-a")];
        let g = gw(providers, &bindings);
        let mut req = GatewayRequest::new(Role::Executor, Capability::Text);
        req.messages.push(aether_models::Message { role: "user".into(), content: "hi".into(), tool_call_id: None, tool_calls: None }.into());
        let resp = g.complete(req).await.unwrap();
        assert_eq!(resp.model_key, "nvidia-a");
        assert_eq!(resp.provider_id, "fake");
        assert_eq!(resp.content.as_deref(), Some("ok"));
    }

    #[tokio::test]
    async fn unconfigured_role_errors_not_routed_elsewhere() {
        let providers: HashMap<String, Arc<dyn ModelProvider>> = [(
            "a".to_string(),
            Arc::new(FakeProvider { fail_status: None, delay: Duration::from_millis(1) }) as Arc<dyn ModelProvider>,
        )].into_iter().collect();
        let bindings = vec![RoleBinding::new(Role::Executor, "a")];
        let g = gw(providers, &bindings);
        // Controller role is NOT configured — must error, never fall back to executor.
        let req = GatewayRequest::new(Role::Controller, Capability::Text);
        let err = g.complete(req).await.unwrap_err();
        assert!(matches!(err, GatewayError::NotConfigured(_)));
    }

    #[tokio::test]
    async fn provider_failure_is_classified_and_isolated() {
        let providers: HashMap<String, Arc<dyn ModelProvider>> = [
            (
                "bad".to_string(),
                Arc::new(FakeProvider {
                    fail_status: Some(429),
                    delay: Duration::from_millis(1),
                }) as Arc<dyn ModelProvider>,
            ),
            (
                "good".to_string(),
                Arc::new(FakeProvider { fail_status: None, delay: Duration::from_millis(1) }) as Arc<dyn ModelProvider>,
            ),
        ].into_iter().collect();
        let bindings = vec![
            RoleBinding::new(Role::Executor, "bad"),
            RoleBinding::new(Role::Controller, "good"),
        ];
        let g = gw(providers, &bindings);
        // Executor fails...
        let err = g.complete(GatewayRequest::new(Role::Executor, Capability::Text)).await.unwrap_err();
        assert_eq!(err.class(), FailureClass::RateLimited);
        // ...but Controller on a different provider still works (isolation).
        let ok = g.complete(GatewayRequest::new(Role::Controller, Capability::Text)).await.unwrap();
        assert_eq!(ok.content.as_deref(), Some("ok"));
    }

    #[tokio::test]
    async fn timeout_produces_request_timeout_class() {
        let providers: HashMap<String, Arc<dyn ModelProvider>> = [(
            "slow".to_string(),
            Arc::new(FakeProvider { fail_status: None, delay: Duration::from_secs(5) }) as Arc<dyn ModelProvider>,
        )].into_iter().collect();
        let bindings = vec![RoleBinding::new(Role::Controller, "slow")];
        let g = gw(providers, &bindings);
        let mut req = GatewayRequest::new(Role::Controller, Capability::Text);
        req.timeout = Some(Duration::from_millis(50));
        let err = g.complete(req).await.unwrap_err();
        assert_eq!(err.class(), FailureClass::RequestTimeout);
    }

    #[tokio::test]
    async fn cancellation_aborts_in_flight() {
        let providers: HashMap<String, Arc<dyn ModelProvider>> = [(
            "slow".to_string(),
            Arc::new(FakeProvider { fail_status: None, delay: Duration::from_secs(5) }) as Arc<dyn ModelProvider>,
        )].into_iter().collect();
        let bindings = vec![RoleBinding::new(Role::Controller, "slow")];
        let g = gw(providers, &bindings);
        let handle = g.cancel_handle();
        let g2 = g.clone();
        let fut = async move {
            let req = GatewayRequest::new(Role::Controller, Capability::Text);
            g2.complete(req).await
        };
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            handle.notify_waiters();
        });
        let err = fut.await.unwrap_err();
        assert!(matches!(err, GatewayError::Cancelled(_)));
    }

    #[tokio::test]
    async fn vision_precheck_denies_without_capability() {
        let providers: HashMap<String, Arc<dyn ModelProvider>> = [(
            "textonly".to_string(),
            Arc::new(FakeProvider { fail_status: None, delay: Duration::from_millis(1) }) as Arc<dyn ModelProvider>,
        )].into_iter().collect();
        let bindings = vec![RoleBinding::new(Role::Reviewer, "textonly")];
        let g = gw(providers, &bindings);
        let mut req = GatewayRequest::new(Role::Reviewer, Capability::Text);
        req.images = Some(vec!["data:image/png;base64,x".into()]);
        let err = g.complete(req).await.unwrap_err();
        assert_eq!(err.class(), FailureClass::UnsupportedCapability);
    }

    #[test]
    fn classify_maps_provider_errors() {
        let e = classify_provider_error(ProviderError::ApiStatus { status: 401, body: "bad key".into() });
        assert_eq!(e.class(), FailureClass::InvalidApiKey);
        let e2 = classify_provider_error(ProviderError::MissingEnv("K".into()));
        assert_eq!(e2.class(), FailureClass::InvalidApiKey);
    }
}
