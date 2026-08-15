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
        // Bound the network call so a hung upstream cannot stall the CLI indefinitely. Without
        // these, `reqwest::Client::new()` has no timeout and a dropped connection can hang
        // until the OS TCP timeout (minutes), tying up the LLM loop and the visual loop.
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(ProviderError::Http)?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            default_model: default_model.to_string(),
            client,
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
        // Defend against SSRF / exfiltration by allowing only safe schemes and capping length.
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
                                "image_url": { "url": sanitize_image_url(url) }
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

/// Validate an image URL before sending it to a vision model. Only `https:` and `data:` schemes
/// are accepted (defense against SSRF / internal-endpoint exfiltration). `data:` URLs must be
/// images. Length is capped to keep request bodies bounded.
fn sanitize_image_url(url: &str) -> String {
    const MAX_LEN: usize = 20 * 1024 * 1024; // 20 MiB per image (generous for screenshots).
    if url.len() > MAX_LEN {
        // Replace an oversized URL with a tiny 1x1 transparent PNG so the request still parses
        // but the model is not asked to ingest a multi-MB image. We surface the truncation
        // through a sentinel string in the text part elsewhere if needed.
        return "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=".to_string();
    }
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("https://") {
        return url.to_string();
    }
    if let Some(rest) = lower.strip_prefix("data:") {
        // data:[<mediatype>][;base64],<data>
        if rest.starts_with("image/") {
            return url.to_string();
        }
    }
    // Unknown / unsafe scheme — return a blank data: image rather than the raw value so the
    // model never sees a URL that could exfiltrate or trigger an internal request.
    "data:image/png;base64,".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Message;

    fn test_provider() -> OpenAICompatibleProvider {
        // `new()` reads the API key from the environment, which is only used at request time.
        // Set a dummy so the constructor succeeds in test isolation.
        std::env::set_var("OPENAI_API_KEY", "sk-test-dummy");
        OpenAICompatibleProvider::new("https://x", "OPENAI_API_KEY", "m", None).unwrap()
    }

    fn req_with_images(images: Vec<String>) -> CompletionRequest {
        CompletionRequest {
            model: "m".into(),
            messages: vec![Message { role: "user".into(), content: "look".into(), ..Default::default() }],
            images: Some(images),
            ..Default::default()
        }
    }

    #[test]
    fn build_body_passes_through_https_and_data() {
        let p = test_provider();
        let req = req_with_images(vec![
            "https://example.com/a.png".into(),
            "data:image/png;base64,iVBORw0K".into(),
        ]);
        let body = p.build_body(&req);
        let arr = body["messages"].as_array().unwrap();
        let content = arr[0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["image_url"]["url"], "https://example.com/a.png");
        assert_eq!(content[2]["image_url"]["url"], "data:image/png;base64,iVBORw0K");
    }

    #[test]
    fn build_body_strips_unsafe_schemes() {
        let p = test_provider();
        let req = req_with_images(vec![
            "http://internal/s".into(),                 // cleartext
            "file:///etc/passwd".into(),                 // filesystem
            "javascript:alert(1)".into(),               // script
            "data:text/html,<script>".into(),            // wrong media type
        ]);
        let body = p.build_body(&req);
        let content = body["messages"][0]["content"].as_array().unwrap();
        // All four are replaced with a blank data: image — nothing unsafe leaks to the model.
        for part in &content[1..] {
            assert_eq!(part["image_url"]["url"], "data:image/png;base64,");
        }
    }

    #[test]
    fn sanitize_caps_oversized_payload() {
        let huge = format!("data:image/png;base64,{}", "A".repeat(21 * 1024 * 1024));
        let s = sanitize_image_url(&huge);
        assert!(s.len() < 200, "oversized image must be replaced, not echoed");
    }
}
