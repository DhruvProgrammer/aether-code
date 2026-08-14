//! Extraction pipeline (spec §9.3). Opt-in: after a task, ask the model to extract
//! structured memory (entities + relations) as JSON and persist into the graph.
//! Failures are swallowed — extraction is best-effort and never blocks the user.

use super::Mind;
use aether_models::{CompletionRequest, Message, ModelProvider};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Extracted {
    #[serde(default)]
    nodes: Vec<ExtractedNode>,
    #[serde(default)]
    edges: Vec<ExtractedEdge>,
}

#[derive(Debug, Deserialize)]
struct ExtractedNode {
    kind: Option<String>,
    label: String,
    #[serde(default)]
    importance: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct ExtractedEdge {
    source: String,
    target: String,
    relation: Option<String>,
    #[serde(default)]
    confidence: Option<f64>,
}

pub async fn extract(
    mind: &Mind,
    text: &str,
    provider: &dyn ModelProvider,
    model: &str,
) -> anyhow::Result<()> {
    let system = "You extract durable facts from an agent's task transcript. \
                  Return ONLY strict JSON: {\"nodes\":[{\"kind\":\"user|project|episodic\",\"label\":string,\"importance\":0..1}],\
                  \"edges\":[{\"source\":label,\"target\":label,\"relation\":string,\"confidence\":0..1}]}. \
                  Capture user preferences, project facts, and decisions. Ignore ephemeral chatter.";
    let req = CompletionRequest {
        model: model.to_string(),
        messages: vec![
            Message { role: "system".into(), content: system.into(), ..Default::default() },
            Message { role: "user".into(), content: text.to_string(), ..Default::default() },
        ],
        ..Default::default()
    };
    let resp = match provider.complete(req).await {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };
    let content = resp.content.unwrap_or_default();
    let parsed: Extracted = match serde_json::from_str(&content) {
        Ok(p) => p,
        Err(_) => return Ok(()),
    };

    let mut id_by_label: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for n in &parsed.nodes {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let node = super::Node {
            id: id.clone(),
            kind: n.kind.clone().unwrap_or_else(|| "episodic".into()),
            label: n.label.clone(),
            importance: n.importance.unwrap_or(0.5) as f32,
            created_at: now,
        };
        let _ = mind.save_node(&node);
        id_by_label.insert(n.label.clone(), id);
    }
    for e in &parsed.edges {
        let source = id_by_label.get(&e.source).cloned().unwrap_or_else(|| e.source.clone());
        let target = id_by_label.get(&e.target).cloned().unwrap_or_else(|| e.target.clone());
        let edge = super::Edge {
            id: uuid::Uuid::new_v4().to_string(),
            source,
            target,
            relation: e.relation.clone().unwrap_or_else(|| "related".into()),
            valid_from: chrono::Utc::now().to_rfc3339(),
            valid_until: None,
            confidence: e.confidence.unwrap_or(0.7) as f32,
        };
        let _ = mind.save_edge(&edge);
    }
    Ok(())
}
