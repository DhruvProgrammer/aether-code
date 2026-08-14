//! Model provider abstraction (spec §6). OpenAI-compatible is the only required v1 backend.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

mod openai;
pub use openai::OpenAICompatibleProvider;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

#[derive(Debug, Clone, Default)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub stream: bool,
    pub tools: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Default)]
pub struct CompletionResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<Usage>,
}

pub type TokenStream = futures_util::stream::BoxStream<'static, Result<String, ProviderError>>;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("provider api error: {0}")]
    Api(String),
    #[error("missing environment variable: {0}")]
    MissingEnv(String),
}

#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError>;
    async fn stream(&self, req: CompletionRequest) -> Result<TokenStream, ProviderError>;
    async fn embeddings(&self, input: Vec<String>) -> Result<Vec<Vec<f32>>, ProviderError>;
    fn supports_tool_calling(&self) -> bool;
    fn name(&self) -> &str;
}

/// Build a provider from a `ModelConfig`. Only `openai_compatible` is supported in v1.
pub fn build_provider(cfg: &aether_config::ModelConfig) -> Result<Box<dyn ModelProvider>, ProviderError> {
    match cfg.provider.as_str() {
        "openai_compatible" => Ok(Box::new(OpenAICompatibleProvider::from_config(cfg)?)),
        other => Err(ProviderError::Api(format!("unsupported provider: {other}"))),
    }
}
