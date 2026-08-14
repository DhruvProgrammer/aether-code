//! `aether-mcp` — exposes `aether-mind` (memory + skills) as an MCP server over stdio
//! JSON-RPC (spec §6, Phase 6). No LLM needed to serve memory; embeddings are optional.

use std::collections::HashMap;
use std::sync::Arc;

use aether_config::Config;
use aether_mind::{skills::SkillIndex, tools as mind_tools, Mind};
use aether_models::ModelProvider;
use aether_tools::{Tool, ToolContext};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub async fn run() -> anyhow::Result<()> {
    let cfg = Config::load(None)?;

    let mind_path = aether_config::expand_tilde(&cfg.memory.path);
    let mind = Mind::open(&mind_path)?;
    let embedder: Option<Arc<dyn ModelProvider>> = cfg
        .model(&cfg.agent.controller_model)
        .and_then(|c| aether_models::build_provider(c).ok())
        .map(Arc::from);

    let skills = SkillIndex::discover(&std::env::current_dir()?);

    let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
    for t in mind_tools::memory_tools(mind.clone(), embedder) {
        tools.insert(t.name().to_string(), t);
    }
    for t in mind_tools::skill_tools(skills) {
        tools.insert(t.name().to_string(), t);
    }

    let cwd = std::env::current_dir()?;
    let stdin = tokio::io::stdin();
    let mut out = tokio::io::stdout();
    let mut reader = BufReader::new(stdin).lines();

    while let Some(line) = reader.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let id = req.get("id").cloned();
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(Value::Null);

        if let Some(resp) = handle(method, &params, &tools, &cwd).await {
            let mut obj = serde_json::json!({ "jsonrpc": "2.0" });
            if let Some(i) = id {
                obj["id"] = i;
            }
            if let Some(r) = resp.get("result") {
                obj["result"] = r.clone();
            }
            if let Some(e) = resp.get("error") {
                obj["error"] = e.clone();
            }
            let s = serde_json::to_string(&obj)?;
            out.write_all(s.as_bytes()).await?;
            out.write_all(b"\n").await?;
            out.flush().await?;
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("aether-mcp error: {e}");
        std::process::exit(1);
    }
}

async fn handle(
    method: &str,
    params: &Value,
    tools: &HashMap<String, Arc<dyn Tool>>,
    cwd: &std::path::Path,
) -> Option<Value> {
    match method {
        "initialize" => Some(serde_json::json!({
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "aether-mind", "version": "0.1.0" }
            }
        })),
        "ping" => Some(serde_json::json!({ "result": {} })),
        "notifications/initialized" => None,
        "tools/list" => {
            let list: Vec<Value> = tools
                .values()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name(),
                        "description": t.description(),
                        "inputSchema": t.json_schema()
                    })
                })
                .collect();
            Some(serde_json::json!({ "result": { "tools": list } }))
        }
        "tools/call" => {
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            match tools.get(name) {
                Some(t) => {
                    let ctx = ToolContext { cwd: cwd.to_path_buf() };
                    let res = t.execute(args, &ctx).await;
                    let (text, is_error) = match res {
                        Ok(r) => (r.output, r.is_error),
                        Err(e) => (e.to_string(), true),
                    };
                    Some(serde_json::json!({
                        "result": {
                            "content": [{ "type": "text", "text": text }],
                            "isError": is_error
                        }
                    }))
                }
                None => Some(serde_json::json!({
                    "error": { "code": -32601, "message": format!("unknown tool: {name}") }
                })),
            }
        }
        _ => Some(serde_json::json!({
            "error": { "code": -32601, "message": format!("method not found: {method}") }
        })),
    }
}
