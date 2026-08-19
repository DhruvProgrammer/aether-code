//! Normalized request/response format (gateway spec §9).
//!
//! AETHER components build a [`GatewayRequest`] — they never construct
//! provider-specific bodies. The gateway translates it into the configured
//! adapter's wire format and returns a [`GatewayResponse`].
//!
//! Every request declares the [`Capability`] it needs so the gateway can
//! pre-validate against the *explicitly configured* model (spec §7) and fail
//! with `unsupported_capability` instead of silently switching.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use aether_models::{CompletionRequest, CompletionResponse, Message, ToolCall, Usage};

use crate::role::Role;

/// The operation a request requires from the configured model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Plain text completion. Every model supports this.
    Text,
    /// Tool/function calling.
    ToolCalling,
    /// Image (multimodal) input.
    Vision,
    /// Streaming token output.
    Streaming,
}

/// A single message in a gateway request. Thin mirror of the wire message so
/// the normalized layer stays provider-neutral.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
}

impl From<Message> for GatewayMessage {
    fn from(m: Message) -> Self {
        Self { role: m.role, content: m.content, tool_call_id: m.tool_call_id, tool_calls: m.tool_calls }
    }
}

impl From<GatewayMessage> for Message {
    fn from(m: GatewayMessage) -> Self {
        Self { role: m.role, content: m.content, tool_call_id: m.tool_call_id, tool_calls: m.tool_calls }
    }
}

/// Normalized request. The gateway fills in the model from the role binding —
/// callers never set it.
#[derive(Debug, Clone)]
pub struct GatewayRequest {
    /// Which AETHER role is requesting. Determines the provider/model.
    pub role: Role,
    pub messages: Vec<GatewayMessage>,
    pub tools: Option<Vec<serde_json::Value>>,
    pub images: Option<Vec<String>>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub stream: bool,
    /// Required capability; validated before dispatch (never auto-switched).
    pub required: Capability,
    /// Per-request timeout override. None → gateway default.
    pub timeout: Option<Duration>,
    /// Free-form metadata; echoed back in the response and in tracing.
    pub metadata: std::collections::HashMap<String, String>,
}

impl GatewayRequest {
    pub fn new(role: Role, required: Capability) -> Self {
        Self {
            role,
            messages: Vec::new(),
            tools: None,
            images: None,
            temperature: None,
            max_tokens: None,
            stream: false,
            required,
            timeout: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Convert into the provider layer's request with the resolved model id.
    pub fn into_completion(self, model: String) -> CompletionRequest {
        CompletionRequest {
            model,
            messages: self.messages.into_iter().map(Message::from).collect(),
            temperature: self.temperature,
            max_tokens: self.max_tokens,
            stream: self.stream,
            tools: self.tools,
            images: self.images,
        }
    }
}

/// Normalized response. Carries which provider/model actually served it so
/// observability can show "what was used" without exposing credentials
/// (spec §18).
#[derive(Debug, Clone)]
pub struct GatewayResponse {
    pub role: Role,
    /// Model key from config (not a secret). E.g. "nvidia-a".
    pub model_key: String,
    /// The concrete model id the adapter called. E.g. "meta/llama-3.1-70b".
    pub model_id: String,
    /// Adapter/protocol id, e.g. "openai_compatible".
    pub provider_id: String,
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<Usage>,
    /// Wall-clock latency for the call.
    pub latency_ms: u64,
}

impl GatewayResponse {
    pub fn from_completion(role: Role, model_key: String, model_id: String, provider_id: String, c: CompletionResponse) -> Self {
        Self {
            role,
            model_key,
            model_id,
            provider_id,
            content: c.content,
            tool_calls: c.tool_calls,
            usage: c.usage,
            latency_ms: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_carries_role_and_capability() {
        let mut r = GatewayRequest::new(Role::Controller, Capability::Text);
        r.messages.push(GatewayMessage { role: "user".into(), content: "hi".into(), tool_call_id: None, tool_calls: None });
        assert_eq!(r.role, Role::Controller);
        assert_eq!(r.required, Capability::Text);
        assert!(!r.stream);
    }

    #[test]
    fn into_completion_fills_model() {
        let r = GatewayRequest::new(Role::Executor, Capability::ToolCalling);
        let cr = r.into_completion("openrouter-b".into());
        assert_eq!(cr.model, "openrouter-b");
        assert!(!cr.stream);
    }

    #[test]
    fn message_roundtrip() {
        let gm = GatewayMessage { role: "assistant".into(), content: "x".into(), tool_call_id: None, tool_calls: None };
        let m: Message = gm.clone().into();
        let back: GatewayMessage = m.into();
        assert_eq!(back.role, "assistant");
        assert_eq!(back.content, "x");
    }

    #[test]
    fn response_from_completion_maps_fields() {
        let c = CompletionResponse {
            content: Some("ok".into()),
            tool_calls: vec![ToolCall { id: "1".into(), name: "f".into(), arguments: serde_json::json!({}) }],
            usage: Some(Usage { prompt_tokens: 3, completion_tokens: 7 }),
        };
        let r = GatewayResponse::from_completion(Role::Reviewer, "tok-c".into(), "gemini".into(), "openai_compatible".into(), c);
        assert_eq!(r.content.as_deref(), Some("ok"));
        assert_eq!(r.tool_calls.len(), 1);
        assert_eq!(r.model_id, "gemini");
        assert_eq!(r.provider_id, "openai_compatible");
    }
}
