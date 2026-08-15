//! OpenAI-compatible provider: speaks `/v1/chat/completions` and `/v1/embeddings`.

use super::{
    CompletionRequest, CompletionResponse, ModelProvider, ProviderError, ToolCall, TokenStream, Usage,
};
use async_trait::async_trait;
use futures_util::stream::{self, BoxStream};
use serde_json::Value;

pub struct OpenAICompatibleProvider {
    base_url: String,
    api_key: String,
    #[allow(dead_code)]
    default_model: String,
    client: reqwest::Client,
    default_temperature: f32,
    default_max_tokens: u32,
    extra_body: Option<Value>,
}

impl OpenAICompatibleProvider {
    pub fn new(
        base_url: &str,
        api_key_env: &str,
        default_model: &str,
        extra_body: Option<Value>,
    ) -> Result<Self, ProviderError> {
        let api_key = std::env::var(api_key_env)
            .map_err(|_| ProviderError::MissingEnv(api_key_env.to_string()))?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            default_model: default_model.to_string(),
            client: reqwest::Client::new(),
            default_temperature: 0.2,
            default_max_tokens: 4096,
            extra_body,
        })
    }

    pub fn from_config(cfg: &aether_config::ModelConfig) -> Result<Self, ProviderError> {
        Self::new(&cfg.base_url, &cfg.api_key_env, &cfg.model, cfg.extra_body.clone())
    }

    fn build_body(&self, req: &CompletionRequest) -> Value {
        let mut body = serde_json::json!({
            "model": req.model,
            "messages": req.messages,
            "temperature": req.temperature.unwrap_or(self.default_temperature),
            "max_tokens": req.max_tokens.unwrap_or(self.default_max_tokens),
            "stream": req.stream,
        });
        if let Some(tools) = &req.tools {
            body["tools"] = serde_json::json!(tools);
        }
        if let Some(eb) = &self.extra_body {
            merge_json(&mut body, eb);
        }
        // Multimodal: extend the last `user` message with image parts (spec: LLM 3 vision).
        if let Some(imgs) = &req.images {
            if !imgs.is_empty() {
                if let Some(arr) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
                    if let Some(idx) = arr.iter().rposition(|m| m.get("role").and_then(|r| r.as_str()) == Some("user")) {
                        let text = arr[idx]
                            .get("content")
                            .and_then(|c| c.as_str())
                            .unwrap_or("")
                            .to_string();
                        let mut parts = vec![serde_json::json!({ "type": "text", "text": text })];
                        for url in imgs {
                            parts.push(serde_json::json!({
                                "type": "image_url",
                                "image_url": { "url": url }
                            }));
                        }
                        arr[idx]["content"] = serde_json::Value::Array(parts);
                    }
                }
            }
        }
        body
    }
}

#[async_trait]
impl ModelProvider for OpenAICompatibleProvider {
    fn name(&self) -> &str {
        "openai_compatible"
    }

    fn supports_tool_calling(&self) -> bool {
        true
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let body = self.build_body(&req);
        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Api(txt));
        }
        let v: Value = resp.json().await?;
        Ok(parse_completion(&v))
    }

    async fn stream(&self, req: CompletionRequest) -> Result<TokenStream, ProviderError> {
        let mut body = self.build_body(&req);
        body["stream"] = serde_json::Value::Bool(true);
        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Api(txt));
        }
        let bytes = resp.bytes().await?;
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<Result<String, ProviderError>> = parse_sse_text(&text);
        let s: BoxStream<Result<String, ProviderError>> = Box::pin(stream::iter(lines));
        Ok(Box::pin(s))
    }

    async fn embeddings(&self, input: Vec<String>) -> Result<Vec<Vec<f32>>, ProviderError> {
        let body = serde_json::json!({ "model": self.default_model, "input": input });
        let resp = self
            .client
            .post(format!("{}/embeddings", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let txt = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Api(txt));
        }
        let v: Value = resp.json().await?;
        let mut out = Vec::new();
        if let Some(arr) = v["data"].as_array() {
            for item in arr {
                if let Some(emb) = item["embedding"].as_array() {
                    let vec: Vec<f32> = emb.iter().filter_map(|x| x.as_f64().map(|f| f as f32)).collect();
                    out.push(vec);
                }
            }
        }
        Ok(out)
    }
}

fn merge_json(base: &mut Value, overlay: &Value) {
    if let (Some(b), Some(o)) = (base.as_object_mut(), overlay.as_object()) {
        for (k, v) in o {
            b.insert(k.clone(), v.clone());
        }
    }
}

fn parse_completion(v: &Value) -> CompletionResponse {
    let mut resp = CompletionResponse::default();
    let choice = &v["choices"][0];
    if let Some(content) = choice["message"]["content"].as_str() {
        resp.content = Some(content.to_string());
    }
    if let Some(tool_calls) = choice["message"]["tool_calls"].as_array() {
        for tc in tool_calls {
            let id = tc["id"].as_str().unwrap_or("").to_string();
            let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
            let args: Value = serde_json::from_str(
                tc["function"]["arguments"].as_str().unwrap_or("{}"),
            )
            .unwrap_or(Value::Null);
            resp.tool_calls.push(ToolCall { id, name, arguments: args });
        }
    }
    if let Some(u) = v["usage"].as_object() {
        resp.usage = Some(Usage {
            prompt_tokens: u.get("prompt_tokens").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
            completion_tokens: u.get("completion_tokens").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
        });
    }
    resp
}

fn parse_sse_text(text: &str) -> Vec<Result<String, ProviderError>> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("data:") {
            continue;
        }
        let data = line.trim_start_matches("data:").trim();
        if data == "[DONE]" {
            continue;
        }
        match serde_json::from_str::<Value>(data) {
            Ok(v) => {
                if let Some(c) = v["choices"].get(0).and_then(|c| c["delta"]["content"].as_str()) {
                    if !c.is_empty() {
                        out.push(Ok(c.to_string()));
                    }
                }
            }
            Err(e) => out.push(Err(ProviderError::Json(e))),
        }
    }
    out
}
