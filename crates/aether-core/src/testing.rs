//! Mock implementations for integration tests.
//!
//! `MockProvider` is a deterministic `ModelProvider` that records every
//! request and replies with a scripted sequence of responses. This is
//! essential for integration tests of the agent loop that must not hit the
//! network. `MockWorkspace` is an in-memory `Workspace` used to exercise
//! tool paths without touching the filesystem.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use aether_models::{CompletionRequest, CompletionResponse, Message, ModelProvider, ProviderError, TokenStream};
use aether_tools::workspace::{FileEntry, FileMeta, ReadResult, SearchHit, Workspace, WorkspaceError, WorkspaceResult};
use async_trait::async_trait;
use futures_util::stream;
use serde_json::Value;

/// One scripted reply for a `MockProvider`. `content` is returned as the
/// assistant text; `tool_calls` is returned as-is when `Some`.
#[derive(Debug, Clone)]
pub struct MockResponse {
    pub content: String,
    pub tool_calls: Option<Vec<Value>>,
    pub finish_reason: Option<String>,
}

impl MockResponse {
    pub fn text(s: impl Into<String>) -> Self {
        Self { content: s.into(), tool_calls: None, finish_reason: Some("stop".into()) }
    }
    pub fn tool_call(name: &str, args: Value) -> Self {
        Self {
            content: String::new(),
            tool_calls: Some(vec![serde_json::json!({
                "id": format!("call_{name}_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)),
                "type": "function",
                "function": { "name": name, "arguments": args.to_string() }
            })]),
            finish_reason: Some("tool_calls".into()),
        }
    }
    pub fn done() -> Self {
        Self { content: String::new(), tool_calls: None, finish_reason: Some("stop".into()) }
    }
}

struct RecordedRequest {
    model: String,
    messages: Vec<Message>,
    estimated_input_tokens: u32,
}

#[derive(Clone)]
struct RecordedRequestInner {
    model: String,
    messages: Vec<Message>,
    estimated_input_tokens: u32,
}

#[derive(Default)]
pub struct MockProvider {
    name: String,
    responses: Mutex<Vec<MockResponse>>,
    recorded: Mutex<Vec<RecordedRequestInner>>,
}

impl MockProvider {
    pub fn new(responses: Vec<MockResponse>) -> Self {
        Self { name: "mock".into(), responses: Mutex::new(responses), recorded: Mutex::new(Vec::new()) }
    }
    pub fn recorded(&self) -> Vec<RecordedRequestPublic> {
        self.recorded
            .lock()
            .unwrap()
            .iter()
            .map(|r| RecordedRequestPublic {
                model: r.model.clone(),
                estimated_input_tokens: r.estimated_input_tokens,
                message_count: r.messages.len(),
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct RecordedRequestPublic {
    pub model: String,
    pub estimated_input_tokens: u32,
    pub message_count: usize,
}

#[async_trait]
impl ModelProvider for MockProvider {
    fn name(&self) -> &str { &self.name }
    fn supports_tool_calling(&self) -> bool { true }
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let mut rec = self.recorded.lock().unwrap();
        rec.push(RecordedRequestInner {
            model: req.model.clone(),
            estimated_input_tokens: req.messages.iter().map(|m| m.content.chars().count() as u32 / 4 + 16).sum(),
            messages: req.messages.clone(),
        });
        drop(rec);
        let mut q = self.responses.lock().unwrap();
        let next = if q.is_empty() { MockResponse::done() } else { q.remove(0) };
        Ok(CompletionResponse {
            content: Some(next.content),
            tool_calls: next.tool_calls.unwrap_or_default().into_iter().filter_map(|v| {
                let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let name = v.get("function").and_then(|f| f.get("name")).and_then(|n| n.as_str()).unwrap_or("").to_string();
                let args = v.get("function").and_then(|f| f.get("arguments")).and_then(|a| a.as_str()).and_then(|s| serde_json::from_str::<Value>(s).ok()).unwrap_or(Value::Null);
                let call = aether_models::ToolCall { id, name, arguments: args };
                Some(call)
            }).collect(),
            usage: None,
        })
    }
    async fn stream(&self, req: CompletionRequest) -> Result<TokenStream, ProviderError> {
        let s = self.complete(req).await?;
        let text = s.content.unwrap_or_default();
        let items: Vec<Result<String, ProviderError>> = text
            .chars()
            .map(|c| Ok(c.to_string()))
            .collect();
        let stream: TokenStream = Box::pin(stream::iter(items));
        Ok(stream)
    }
    async fn embeddings(&self, _input: Vec<String>) -> Result<Vec<Vec<f32>>, ProviderError> {
        Ok(vec![vec![0.0; 8]])
    }
}

#[derive(Debug, Default)]
pub struct MockWorkspace {
    root: PathBuf,
    files: Mutex<HashMap<String, String>>,
}

impl MockWorkspace {
    pub fn new(root: PathBuf) -> Self {
        Self { root, files: Mutex::new(HashMap::new()) }
    }
    pub fn seed(&self, path: &str, content: &str) {
        // store under the relative key (root prefix is dropped if absolute)
        let key = if std::path::Path::new(path).is_absolute() {
            self.root.join(path).strip_prefix(&self.root).unwrap_or(std::path::Path::new(path)).to_string_lossy().to_string()
        } else {
            path.to_string()
        };
        eprintln!("[mock-ws] seed path='{}' key='{}' content_len={}", path, key, content.len());
        self.files.lock().unwrap().insert(key, content.to_string());
    }
    pub fn snapshot(&self) -> std::collections::HashMap<String, String> {
        self.files.lock().unwrap().clone()
    }
}

#[async_trait]
impl Workspace for MockWorkspace {
    fn root(&self) -> &Path { &self.root }
        async fn read(&self, rel: &Path, _offset: u32, _max_lines: u32, max_bytes: u32) -> WorkspaceResult<ReadResult> {
        let key = rel.to_string_lossy().to_string();
        let content = self.files.lock().unwrap().get(&key).cloned().unwrap_or_default();
        // max_bytes == 0 means no cap
        let truncated = max_bytes > 0 && content.len() > max_bytes as usize;
        let text = if truncated { content[..max_bytes as usize].to_string() } else { content };
        Ok(ReadResult { text, truncated, truncated_lines: 0 })
    }
    async fn write(&self, rel: &Path, content: &str) -> WorkspaceResult<()> {
        self.files.lock().unwrap().insert(rel.to_string_lossy().to_string(), content.to_string());
        Ok(())
    }
    async fn replace_in_file(&self, rel: &Path, old: &str, new: &str, _all: bool) -> WorkspaceResult<()> {
        let k = rel.to_string_lossy().to_string();
        let mut f = self.files.lock().unwrap();
        let s = f.get(&k).cloned().ok_or_else(|| WorkspaceError::NotFound(k.clone()))?;
        f.insert(k, s.replace(old, new));
        Ok(())
    }
    async fn list(&self, _rel: &Path) -> WorkspaceResult<Vec<FileEntry>> {
        Ok(self.files.lock().unwrap().keys().map(|p| FileEntry {
            rel: PathBuf::from(p),
            meta: FileMeta { size: 0, mtime_ms: 0, is_dir: false, readonly: false },
        }).collect())
    }
    async fn stat(&self, rel: &Path) -> WorkspaceResult<Option<FileMeta>> {
        if self.files.lock().unwrap().contains_key(&rel.to_string_lossy().to_string()) {
            Ok(Some(FileMeta { size: 0, mtime_ms: 0, is_dir: false, readonly: false }))
        } else { Ok(None) }
    }
    async fn find_files(&self, name: &str, _max: usize) -> WorkspaceResult<Vec<std::path::PathBuf>> {
        Ok(self.files.lock().unwrap().keys().filter(|k| k.ends_with(name) || k.contains(name)).map(std::path::PathBuf::from).collect())
    }
    async fn search_content(&self, query: &str, _max: usize) -> WorkspaceResult<Vec<SearchHit>> {
        let mut out = Vec::new();
        let needle = query.to_ascii_lowercase();
        for (k, v) in self.files.lock().unwrap().iter() {
            for (i, line) in v.lines().enumerate() {
                if line.to_ascii_lowercase().contains(&needle) {
                    out.push(SearchHit { rel: PathBuf::from(k), line_no: (i as u32) + 1, line: line.to_string() });
                }
            }
        }
        Ok(out)
    }
}
