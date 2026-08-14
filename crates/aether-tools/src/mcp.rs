//! MCP client (spec §6, Phase 6). Connects to an external MCP server over stdio
//! JSON-RPC and adapts its tools into `aether_tools::Tool` so they appear in the agent's
//! toolset like any local tool.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use aether_permissions::Permission;
use crate::{Tool, ToolContext, ToolError, ToolResult};
use anyhow::anyhow;
use async_trait::async_trait;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{mpsc, Mutex};

#[derive(Debug, Clone)]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

pub struct McpClient {
    stdin: Mutex<ChildStdin>,
    rx: Mutex<mpsc::UnboundedReceiver<Value>>,
    next_id: AtomicU64,
    buf: Mutex<Vec<Value>>,
}

impl McpClient {
    pub async fn connect(command: &str, args: &[String]) -> anyhow::Result<Arc<Self>> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("mcp: no stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("mcp: no stdout"))?;

        let (tx, rx) = mpsc::unbounded_channel::<Value>();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    let _ = tx.send(v);
                }
            }
        });

        let client = Arc::new(Self {
            stdin: Mutex::new(stdin),
            rx: Mutex::new(rx),
            next_id: AtomicU64::new(1),
            buf: Mutex::new(Vec::new()),
        });

        client
            .request("initialize", serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "aether", "version": "0.1.0" }
            }))
            .await?;
        client.notify("notifications/initialized").await?;
        Ok(client)
    }

    pub async fn list_tools(&self) -> anyhow::Result<Vec<McpToolInfo>> {
        let resp = self.request("tools/list", serde_json::json!({})).await?;
        let arr = resp
            .get("result")
            .and_then(|r| r.get("tools"))
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for t in arr {
            out.push(McpToolInfo {
                name: t.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                description: t.get("description").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                schema: t
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({ "type": "object" })),
            });
        }
        Ok(out)
    }

    pub async fn call_tool(&self, name: &str, args: Value) -> anyhow::Result<Value> {
        self.request("tools/call", serde_json::json!({ "name": name, "arguments": args }))
            .await
    }

    async fn request(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        {
            let mut s = self.stdin.lock().await;
            let bytes = serde_json::to_vec(&req)?;
            s.write_all(&bytes).await?;
            s.write_all(b"\n").await?;
            s.flush().await?;
        }
        loop {
            {
                let mut buf = self.buf.lock().await;
                if let Some(pos) = buf
                    .iter()
                    .position(|v| v.get("id").and_then(|x| x.as_u64()) == Some(id))
                {
                    return Ok(buf.remove(pos));
                }
            }
            let mut rx = self.rx.lock().await;
            let v = rx.recv().await.ok_or_else(|| anyhow!("mcp server closed connection"))?;
            if v.get("id").and_then(|x| x.as_u64()) == Some(id) {
                return Ok(v);
            } else {
                self.buf.lock().await.push(v);
            }
        }
    }

    async fn notify(&self, method: &str) -> anyhow::Result<()> {
        let req = serde_json::json!({ "jsonrpc": "2.0", "method": method });
        let mut s = self.stdin.lock().await;
        let bytes = serde_json::to_vec(&req)?;
        s.write_all(&bytes).await?;
        s.write_all(b"\n").await?;
        s.flush().await?;
        Ok(())
    }
}

/// Adapts one remote MCP tool into a local `Tool` (spec §6).
pub struct McpTool {
    client: Arc<McpClient>,
    name: String,
    description: String,
    schema: Value,
}

impl McpTool {
    pub fn from_info(client: Arc<McpClient>, info: McpToolInfo) -> Self {
        Self {
            client,
            name: info.name,
            description: info.description,
            schema: info.schema,
        }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn category(&self) -> &'static str {
        "network"
    }
    fn required_permission(&self) -> Permission {
        Permission::Allow
    }
    fn json_schema(&self) -> Value {
        self.schema.clone()
    }
    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let v = self
            .client
            .call_tool(&self.name, args)
            .await
            .map_err(|e| ToolError::Other(e.to_string()))?;
        let content = v
            .get("result")
            .and_then(|r| r.get("content"))
            .and_then(|c| c.get(0))
            .and_then(|x| x.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        let is_error = v
            .get("result")
            .and_then(|r| r.get("isError"))
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        Ok(ToolResult { output: content, is_error })
    }
}
