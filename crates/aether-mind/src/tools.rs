//! Memory + skills tools (spec §9, §10). Implement `aether_tools::Tool` so they plug
//! into the Executor like any other tool.

use super::{Mind, Node, Edge, skills::SkillIndex};
use aether_models::ModelProvider;
use aether_tools::{Tool, ToolContext, ToolError, ToolResult};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

pub fn memory_tools(mind: Arc<Mind>, embedder: Option<Arc<dyn ModelProvider>>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(MemorySaveTool { mind: mind.clone(), embedder: embedder.clone() }),
        Arc::new(MemoryQueryTool { mind: mind.clone(), embedder }),
        Arc::new(MemoryForgetTool { mind }),
    ]
}

pub fn skill_tools(index: Arc<SkillIndex>) -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(SkillSearchTool { index })]
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

// --------------------------------------------------------------------------
// memory_save
// --------------------------------------------------------------------------

pub struct MemorySaveTool {
    mind: Arc<Mind>,
    embedder: Option<Arc<dyn ModelProvider>>,
}

#[async_trait]
impl Tool for MemorySaveTool {
    fn name(&self) -> &str { "memory_save" }
    fn description(&self) -> &str {
        "Persist a memory node (entity/fact) and optional relations. kind: user|project|episodic|skill. Returns the node id."
    }
    fn category(&self) -> &'static str { "network" }
    fn required_permission(&self) -> aether_permissions::Permission { aether_permissions::Permission::Allow }
    fn json_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "kind": { "type": "string", "description": "user | project | episodic | skill" },
                "label": { "type": "string", "description": "human-readable fact/entity" },
                "importance": { "type": "number", "description": "0..1, default 0.5" },
                "relations": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "target": { "type": "string" },
                            "relation": { "type": "string" },
                            "confidence": { "type": "number" }
                        },
                        "required": ["target", "relation"]
                    }
                }
            },
            "required": ["kind", "label"]
        })
    }
    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let kind = arg_str(&args, "kind").unwrap_or("project").to_string();
        let label = arg_str(&args, "label").ok_or_else(|| ToolError::Other("missing 'label'".into()))?;
        let importance = args.get("importance").and_then(|v| v.as_f64()).unwrap_or(0.5) as f32;
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        let node = Node { id: id.clone(), kind, label: label.to_string(), importance, created_at: now };
        self.mind.save_node(&node)?;

        if let Some(rels) = args.get("relations").and_then(|v| v.as_array()) {
            for r in rels {
                let target = arg_str(r, "target").unwrap_or("").to_string();
                let relation = arg_str(r, "relation").unwrap_or("related").to_string();
                let conf = r.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.7) as f32;
                let edge = Edge {
                    id: uuid::Uuid::new_v4().to_string(),
                    source: id.clone(),
                    target,
                    relation,
                    valid_from: chrono::Utc::now().to_rfc3339(),
                    valid_until: None,
                    confidence: conf,
                };
                self.mind.save_edge(&edge)?;
            }
        }

        if let Some(emb) = self.embedder.as_deref() {
            if let Ok(v) = emb.embeddings(vec![format!("{}: {}", node.kind, node.label)]).await {
                if let Some(vec) = v.into_iter().next() {
                    let (quant, scale) = super::quantize(&vec);
                    let _ = self.mind.save_vector(&super::VectorRow {
                        id: node.id.clone(),
                        kind: node.kind.clone(),
                        text: node.label.clone(),
                        quant,
                        scale,
                    });
                }
            }
        }

        Ok(ToolResult { output: format!("saved node {}", id), is_error: false })
    }
}

// --------------------------------------------------------------------------
// memory_query
// --------------------------------------------------------------------------

pub struct MemoryQueryTool {
    mind: Arc<Mind>,
    embedder: Option<Arc<dyn ModelProvider>>,
}

#[async_trait]
impl Tool for MemoryQueryTool {
    fn name(&self) -> &str { "memory_query" }
    fn description(&self) -> &str { "Retrieve relevant memory (graph + semantic + facts) for a query." }
    fn category(&self) -> &'static str { "network" }
    fn required_permission(&self) -> aether_permissions::Permission { aether_permissions::Permission::Allow }
    fn json_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "k": { "type": "integer", "description": "max nodes, default 5" }
            },
            "required": ["query"]
        })
    }
    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let query = arg_str(&args, "query").ok_or_else(|| ToolError::Other("missing 'query'".into()))?;
        let k = args.get("k").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
        let text = self
            .mind
            .retrieve(query, self.embedder.as_deref(), k)
            .await
            .map_err(|e| ToolError::Other(e.to_string()))?;
        Ok(ToolResult { output: text, is_error: false })
    }
}

// --------------------------------------------------------------------------
// memory_forget
// --------------------------------------------------------------------------

pub struct MemoryForgetTool {
    mind: Arc<Mind>,
}

#[async_trait]
impl Tool for MemoryForgetTool {
    fn name(&self) -> &str { "memory_forget" }
    fn description(&self) -> &str { "Delete a memory node (and its edges) by id." }
    fn category(&self) -> &'static str { "network" }
    fn required_permission(&self) -> aether_permissions::Permission { aether_permissions::Permission::Allow }
    fn json_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": { "id": { "type": "string" } },
            "required": ["id"]
        })
    }
    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let id = arg_str(&args, "id").ok_or_else(|| ToolError::Other("missing 'id'".into()))?;
        self.mind.forget(id)?;
        Ok(ToolResult { output: format!("forgot {id}"), is_error: false })
    }
}

// --------------------------------------------------------------------------
// skill_search
// --------------------------------------------------------------------------

pub struct SkillSearchTool {
    index: Arc<SkillIndex>,
}

#[async_trait]
impl Tool for SkillSearchTool {
    fn name(&self) -> &str { "skill_search" }
    fn description(&self) -> &str { "Search available skills by name/description; returns matching SKILL.md paths." }
    fn category(&self) -> &'static str { "read" }
    fn required_permission(&self) -> aether_permissions::Permission { aether_permissions::Permission::Allow }
    fn json_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": { "query": { "type": "string" } },
            "required": ["query"]
        })
    }
    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let query = arg_str(&args, "query").unwrap_or("");
        let hits = self.index.search(query);
        let text = if hits.is_empty() {
            "no matching skills".to_string()
        } else {
            hits.iter()
                .map(|s| format!("- {}: {} ({})", s.name, s.description, s.path))
                .collect::<Vec<_>>()
                .join("\n")
        };
        Ok(ToolResult { output: text, is_error: false })
    }
}
