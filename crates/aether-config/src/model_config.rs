//! Authoritative model + provider config (OpenCode-inspired depth).
//!
//! AETHER's `ProviderEntry` and `ModelEntry` are persisted on disk. The
//! runtime additionally needs a richer structure (cost, variants, family,
//! status) without breaking the on-disk schema. This module defines the
//! `ModelConfigV2` and `ProviderConfigV2` runtime structs that wrap the
//! on-disk entries and add OpenCode-style fields.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Model status (OpenCode-style).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModelStatus {
    Active,
    Alpha,
    Deprecated,
}

impl Default for ModelStatus {
    fn default() -> Self { ModelStatus::Active }
}

/// Per-token pricing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: f64,
    #[serde(default)]
    pub cache_write: f64,
}

/// Token limits.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelLimit {
    pub context: u32,
    #[serde(default)]
    pub input: u32,
    pub output: u32,
}

/// Modality flags (OpenCode-style).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelModalities {
    #[serde(default = "dft_true")]
    pub text: bool,
    pub audio: bool,
    pub image: bool,
    pub video: bool,
    pub pdf: bool,
}

fn dft_true() -> bool { true }

/// Authoritative runtime model config. `ProviderEntry` + `ModelEntry` from
/// the on-disk schema upgrade into this when the registry loads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfigV2 {
    pub id: String,
    pub provider_id: String,
    pub name: String,
    #[serde(default)]
    pub family: String,
    pub api: ModelApi,
    #[serde(default)]
    pub status: ModelStatus,
    pub limit: ModelLimit,
    pub cost: ModelCost,
    pub capabilities: ModelCapabilities,
    #[serde(default)]
    pub release_date: String,
    #[serde(default)]
    pub variants: HashMap<String, ModelVariant>,
    /// Headers injected into every request to this model.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Free-form model-specific options (forwarded to the SDK).
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelApi {
    /// The model id sent to the API (same as `id` for OpenAI-compatible).
    pub id: String,
    /// SDK npm package name (e.g. "@ai-sdk/openai-compatible"). Optional.
    #[serde(default)]
    pub npm: String,
    /// Base URL (overrides the provider's base URL if set).
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelCapabilities {
    #[serde(default = "dft_true")]
    pub temperature: bool,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub attachment: bool,
    #[serde(default = "dft_true")]
    pub toolcall: bool,
    #[serde(default)]
    pub input: ModelModalities,
    #[serde(default)]
    pub output: ModelModalities,
    #[serde(default)]
    pub interleaved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelVariant {
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

/// Authoritative provider config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfigV2 {
    pub id: String,
    pub display_name: String,
    pub protocol: String,
    pub base_url: String,
    pub auth: ProviderAuth,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    pub models: HashMap<String, ModelConfigV2>,
    /// Environment variables read by the provider (e.g. `OPENAI_API_KEY`).
    #[serde(default)]
    pub env: Vec<String>,
    /// Free-form provider options.
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

/// Provider auth scheme (OpenCode-style).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ProviderAuth {
    /// `Authorization: Bearer <key>`. Key read from env var `name`.
    BearerEnv { name: String },
    /// `Authorization: Bearer <key>` with raw key value.
    BearerRaw { key: String },
    /// `x-api-key: <key>`. Key read from env var `name`.
    ApiKeyEnv { name: String },
    /// `x-api-key: <key>` with raw key.
    ApiKeyRaw { key: String },
    /// No auth.
    None,
}

impl ProviderAuth {
    /// Resolve the actual key value. Returns an error if the env var is
    /// missing.
    pub fn resolve(&self) -> Result<String, String> {
        match self {
            ProviderAuth::BearerEnv { name } | ProviderAuth::ApiKeyEnv { name } => {
                std::env::var(name).map_err(|_| format!("env {name} is not set"))
            }
            ProviderAuth::BearerRaw { key } | ProviderAuth::ApiKeyRaw { key } => Ok(key.clone()),
            ProviderAuth::None => Ok(String::new()),
        }
    }
}

/// Build a runtime `ProviderConfigV2` from the on-disk `ProviderEntry`.
pub fn from_entry(entry: &super::ProviderEntry) -> ProviderConfigV2 {
    let auth = build_auth(&entry.api_key_env, entry.api_key.as_deref(), entry.auth_type.as_deref());
    let models = entry
        .models
        .iter()
        .map(|m| (m.id.clone(), build_model(m, &entry.id)))
        .collect();
    let mut headers = std::collections::HashMap::new();
    if let Some(h) = entry.headers.as_ref().and_then(|v| v.as_object()) {
        for (k, v) in h {
            if let Some(s) = v.as_str() {
                headers.insert(k.clone(), s.to_string());
            }
        }
    }
    let env = vec![entry.api_key_env.clone()];
    ProviderConfigV2 {
        id: entry.id.clone(),
        display_name: if entry.display_name.is_empty() { entry.id.clone() } else { entry.display_name.clone() },
        protocol: entry.protocol.clone(),
        base_url: entry.base_url.clone(),
        auth,
        headers,
        models,
        env,
        options: std::collections::HashMap::new(),
    }
}

fn build_auth(api_key_env: &str, api_key: Option<&str>, auth_type: Option<&str>) -> ProviderAuth {
    match auth_type {
        Some("none") => ProviderAuth::None,
        Some("raw") => ProviderAuth::BearerRaw { key: api_key.unwrap_or("").to_string() },
        Some("env_var") | None => ProviderAuth::BearerEnv { name: api_key_env.to_string() },
        Some(other) => {
            // Unknown variant → fall back to env-var (safest).
            eprintln!("[provider-config] unknown auth_type '{other}', falling back to env_var");
            ProviderAuth::BearerEnv { name: api_key_env.to_string() }
        }
    }
}

fn build_model(m: &super::ModelEntry, provider_id: &str) -> ModelConfigV2 {
    let limit = ModelLimit {
        context: m.context_window.unwrap_or(0),
        input: 0,
        output: m.max_output_tokens.unwrap_or(0),
    };
    let input = ModelModalities { text: true, audio: false, image: m.vision, video: false, pdf: false };
    let output = ModelModalities { text: true, audio: false, image: false, video: false, pdf: false };
    let capabilities = ModelCapabilities {
        temperature: true,
        reasoning: false,
        attachment: m.vision,
        toolcall: m.tool_calling,
        input,
        output,
        interleaved: false,
    };
    ModelConfigV2 {
        id: m.id.clone(),
        provider_id: provider_id.to_string(),
        name: if m.display_name.is_empty() { m.id.clone() } else { m.display_name.clone() },
        family: String::new(),
        api: ModelApi {
            id: m.id.clone(),
            npm: String::new(),
            url: String::new(),
        },
        status: ModelStatus::Active,
        limit,
        cost: ModelCost::default(),
        capabilities,
        release_date: String::new(),
        variants: std::collections::HashMap::new(),
        headers: std::collections::HashMap::new(),
        options: std::collections::HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModelEntry, ProviderEntry};

    fn make_entry(auth_type: Option<&str>, raw_key: Option<&str>) -> ProviderEntry {
        ProviderEntry {
            id: "nvidia".into(),
            display_name: "NVIDIA".into(),
            protocol: "openai_compatible".into(),
            base_url: "https://integrate.api.nvidia.com/v1".into(),
            api_key_env: "NVIDIA_API_KEY".into(),
            auth_type: auth_type.map(|s| s.into()),
            api_key: raw_key.map(|s| s.into()),
            headers: None,
            extra_body: None,
            models: vec![ModelEntry {
                id: "nvidia/llama-3.1-70b".into(),
                display_name: "Llama 3.1 70B".into(),
                vision: false,
                tool_calling: true,
                streaming: true,
                context_window: Some(128000),
                max_output_tokens: None,
            }],
        }
    }

    #[test]
    fn env_var_auth_is_default() {
        let e = make_entry(None, None);
        let p = from_entry(&e);
        assert_eq!(p.id, "nvidia");
        assert!(matches!(p.auth, ProviderAuth::BearerEnv { .. }));
        assert_eq!(p.base_url, "https://integrate.api.nvidia.com/v1");
        assert_eq!(p.models.len(), 1);
    }

    #[test]
    fn raw_key_uses_bearer_raw() {
        let e = make_entry(Some("raw"), Some("nvapi-xxx"));
        let p = from_entry(&e);
        assert!(matches!(p.auth, ProviderAuth::BearerRaw { .. }));
    }

    #[test]
    fn none_auth_means_no_credentials() {
        let e = make_entry(Some("none"), None);
        let p = from_entry(&e);
        assert!(matches!(p.auth, ProviderAuth::None));
    }

    #[test]
    fn model_carries_context_window() {
        let e = make_entry(None, None);
        let p = from_entry(&e);
        let m = p.models.get("nvidia/llama-3.1-70b").unwrap();
        assert_eq!(m.limit.context, 128_000);
    }
}
