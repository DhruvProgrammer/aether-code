//! Provider descriptors.

use serde::{Deserialize, Serialize};

/// Authentication scheme.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AuthConfig {
    /// `Authorization: Bearer <key>`. Key read from env var `name`.
    BearerEnv { name: String },
    /// `Authorization: Bearer <key>` with raw key value (not env var).
    BearerRaw { key: String },
    /// `x-api-key: <key>`. Key read from env var `name`.
    ApiKeyEnv { name: String },
    /// `x-api-key: <key>` with raw key value.
    ApiKeyRaw { key: String },
    /// No auth (local server, Ollama-style).
    None,
}

impl std::fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BearerEnv { name } => f.debug_struct("BearerEnv").field("name", name).finish(),
            Self::BearerRaw { .. } => f.debug_struct("BearerRaw").field("key", &"[REDACTED]").finish(),
            Self::ApiKeyEnv { name } => f.debug_struct("ApiKeyEnv").field("name", name).finish(),
            Self::ApiKeyRaw { .. } => f.debug_struct("ApiKeyRaw").field("key", &"[REDACTED]").finish(),
            Self::None => write!(f, "None"),
        }
    }
}

impl AuthConfig {
    pub fn env_var(&self) -> Option<&str> {
        match self {
            Self::BearerEnv { name } | Self::ApiKeyEnv { name } => Some(name.as_str()),
            Self::BearerRaw { .. } | Self::ApiKeyRaw { .. } | Self::None => None,
        }
    }

    /// Resolve the actual key value, handling env var vs raw.
    pub fn resolve(&self) -> Result<String, String> {
        match self {
            Self::BearerEnv { name } | Self::ApiKeyEnv { name } => {
                std::env::var(name).map_err(|_| format!("env {name} is not set"))
            }
            Self::BearerRaw { key } | Self::ApiKeyRaw { key } => {
                if key.trim().is_empty() {
                    Err("credential is empty".into())
                } else {
                    Ok(key.clone())
                }
            }
            Self::None => Ok(String::new()),
        }
    }

    pub fn is_raw(&self) -> bool {
        matches!(self, Self::BearerRaw { .. } | Self::ApiKeyRaw { .. })
    }
}

/// Live status of a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    Unknown,
    Healthy,
    Degraded,
    Unreachable,
    AuthFailed,
}

/// Full descriptor for an OpenAI-compatible provider (and future non-OpenAI
/// providers; the schema is generic).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDescriptor {
    pub id: String,
    pub display_name: String,
    pub provider_type: String, // "openai_compatible" | future: "anthropic", "google", ...
    pub base_url: String,
    pub auth: AuthConfig,
    pub custom_headers: std::collections::HashMap<String, String>,
    /// Extra env vars to read before a request (e.g. routing keys, org ids).
    pub extra_env: Vec<String>,
    /// Limits applied to all models of this provider.
    pub limits: ProviderLimits,
    /// Pricing (optional, informational).
    pub pricing: Option<Pricing>,
    /// Availability / health.
    pub status: ProviderStatus,
    /// Last health-check latency (ms). `None` until first check.
    pub last_latency_ms: Option<u64>,
    /// Last health-check error message.
    pub last_error: Option<String>,
    /// Models hosted by this provider.
    pub models: Vec<String>, // model ids
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderLimits {
    pub default_temperature: Option<f32>,
    pub default_max_tokens: Option<u32>,
    pub request_timeout_secs: Option<u64>,
    pub max_retries: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pricing {
    pub currency: String,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

impl ProviderDescriptor {
    pub fn new_openai_compatible(
        id: impl Into<String>,
        base_url: impl Into<String>,
        api_key_env: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            display_name: String::new(),
            provider_type: "openai_compatible".into(),
            base_url: base_url.into(),
            auth: AuthConfig::BearerEnv { name: api_key_env.into() },
            custom_headers: Default::default(),
            extra_env: Vec::new(),
            limits: ProviderLimits::default(),
            pricing: None,
            status: ProviderStatus::Unknown,
            last_latency_ms: None,
            last_error: None,
            models: Vec::new(),
        }
    }

    pub fn with_display_name(mut self, n: impl Into<String>) -> Self { self.display_name = n.into(); self }
    pub fn with_header(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.custom_headers.insert(k.into(), v.into());
        self
    }
    pub fn with_model(mut self, model_id: impl Into<String>) -> Self {
        self.models.push(model_id.into());
        self
    }
    pub fn with_status(mut self, s: ProviderStatus) -> Self { self.status = s; self }
}
