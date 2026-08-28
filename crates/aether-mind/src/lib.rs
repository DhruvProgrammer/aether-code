//! `aether-mind` — embedded memory engine (spec §9). Hybrid store:
//!   - graph: nodes + temporal edges (redb)
//!   - semantic: brute-force cosine vector index (redb; swap for `usearch` later)
//!   - kv: flat user/project facts
//! Retrieval fuses vector + keyword + 1-hop graph (spec §9.7).

pub mod context;
pub mod extract;
pub mod skills;
pub mod tools;
pub mod vector;

use anyhow::Result;
use aether_models::ModelProvider;
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

const NODES: TableDefinition<&str, &[u8]> = TableDefinition::new("nodes");
const EDGES: TableDefinition<&str, &[u8]> = TableDefinition::new("edges");
const KV: TableDefinition<&str, &[u8]> = TableDefinition::new("kv");
const VECTORS: TableDefinition<&str, &[u8]> = TableDefinition::new("vectors");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub importance: f32,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub relation: String,
    pub valid_from: String,
    pub valid_until: Option<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VectorRow {
    id: String,
    kind: String,
    text: String,
    /// Scalar-quantized embedding (f32 → i8) to halve storage (spec §9, Phase 6).
    quant: Vec<i8>,
    /// Per-vector scale used during quantization.
    scale: f32,
}

/// Scalar quantize an f32 vector to i8 in [-127, 127] (Phase 6 quantized index).
fn quantize(vec: &[f32]) -> (Vec<i8>, f32) {
    let max_abs = vec.iter().map(|x| x.abs()).fold(0.0_f32, f32::max).max(1e-6);
    let scale = max_abs / 127.0;
    let q = vec.iter().map(|x| (x / scale).round().clamp(-127.0, 127.0) as i8).collect();
    (q, scale)
}

/// Dequantize back to f32 for cosine comparison.
fn dequant(q: &[i8], scale: f32) -> Vec<f32> {
    q.iter().map(|x| *x as f32 * scale).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KvRow {
    key: String,
    value: String,
}

pub mod memory;

pub struct Mind {
    db: Database,
}

impl Mind {
    pub fn open(path: &Path) -> Result<Arc<Self>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Database::create(path)?;
        Ok(Arc::new(Self { db }))
    }

    // --- writes -----------------------------------------------------------

    pub fn save_node(&self, node: &Node) -> Result<()> {
        let bytes = serde_json::to_vec(node)?;
        let txn = self.db.begin_write()?;
        {
            let mut t = txn.open_table(NODES)?;
            t.insert(node.id.as_str(), bytes.as_slice())?;
        }
        txn.commit()?;
        Ok(())
    }

    pub fn save_edge(&self, edge: &Edge) -> Result<()> {
        let bytes = serde_json::to_vec(edge)?;
        let txn = self.db.begin_write()?;
        {
            let mut t = txn.open_table(EDGES)?;
            t.insert(edge.id.as_str(), bytes.as_slice())?;
        }
        txn.commit()?;
        Ok(())
    }

    pub(crate) fn save_vector(&self, row: &VectorRow) -> Result<()> {
        let bytes = serde_json::to_vec(row)?;
        let txn = self.db.begin_write()?;
        {
            let mut t = txn.open_table(VECTORS)?;
            t.insert(row.id.as_str(), bytes.as_slice())?;
        }
        txn.commit()?;
        Ok(())
    }

    pub fn save_kv(&self, key: &str, value: &str) -> Result<()> {
        let row = KvRow { key: key.to_string(), value: value.to_string() };
        let bytes = serde_json::to_vec(&row)?;
        let txn = self.db.begin_write()?;
        {
            let mut t = txn.open_table(KV)?;
            t.insert(key, bytes.as_slice())?;
        }
        txn.commit()?;
        Ok(())
    }

    pub fn forget(&self, id: &str) -> Result<()> {
        let txn = self.db.begin_write()?;
        {
            let mut nodes = txn.open_table(NODES)?;
            nodes.remove(id)?;
            let mut vectors = txn.open_table(VECTORS)?;
            vectors.remove(id)?;
        }
        txn.commit()?;
        // edges referencing this id are removed in a second txn (cannot hold two tables
        // of different types across the same commit boundary easily via borrow rules).
        let txn = self.db.begin_write()?;
        {
            let mut edges = txn.open_table(EDGES)?;
            let to_remove: Vec<String> = edges
                .iter()?
                .filter_map(|r| {
                    let (_, v) = r.ok()?;
                    let e: Edge = serde_json::from_slice(v.value()).ok()?;
                    if e.source == id || e.target == id { Some(e.id) } else { None }
                })
                .collect();
            for eid in to_remove {
                edges.remove(eid.as_str())?;
            }
        }
        txn.commit()?;
        Ok(())
    }

    // --- reads ------------------------------------------------------------

    pub fn get_node(&self, id: &str) -> Result<Option<Node>> {
        let txn = self.db.begin_read()?;
        let t = txn.open_table(NODES)?;
        match t.get(id)? {
            Some(v) => Ok(serde_json::from_slice::<Node>(v.value()).ok()),
            None => Ok(None),
        }
    }

    pub fn list_nodes(&self, limit: usize) -> Result<Vec<Node>> {
        let txn = self.db.begin_read()?;
        let t = txn.open_table(NODES)?;
        let mut out = Vec::new();
        for r in t.iter()? {
            let (_, v) = r?;
            if let Ok(n) = serde_json::from_slice::<Node>(v.value()) {
                out.push(n);
                if out.len() >= limit {
                    break;
                }
            }
        }
        Ok(out)
    }

    fn all_vectors(&self) -> Result<Vec<VectorRow>> {
        let txn = self.db.begin_read()?;
        let t = txn.open_table(VECTORS)?;
        let mut out = Vec::new();
        for r in t.iter()? {
            let (_, v) = r?;
            if let Ok(row) = serde_json::from_slice::<VectorRow>(v.value()) {
                out.push(row);
            }
        }
        Ok(out)
    }

    fn edges_for(&self, source: &str) -> Result<Vec<Edge>> {
        let txn = self.db.begin_read()?;
        let t = txn.open_table(EDGES)?;
        let mut out = Vec::new();
        for r in t.iter()? {
            let (_, v) = r?;
            if let Ok(e) = serde_json::from_slice::<Edge>(v.value()) {
                if e.source == source {
                    out.push(e);
                }
            }
        }
        Ok(out)
    }

    fn all_kv(&self) -> Result<Vec<KvRow>> {
        let txn = self.db.begin_read()?;
        let t = txn.open_table(KV)?;
        let mut out = Vec::new();
        for r in t.iter()? {
            let (_, v) = r?;
            if let Ok(row) = serde_json::from_slice::<KvRow>(v.value()) {
                out.push(row);
            }
        }
        Ok(out)
    }

    /// Hybrid retrieval (spec §9.7). `embedder` enables the vector branch;
    /// without it, retrieval degrades to keyword + graph. Returns a ready-to-inject
    /// text block.
    pub async fn retrieve(
        &self,
        query: &str,
        embedder: Option<&dyn ModelProvider>,
        k: usize,
    ) -> Result<String> {
        let mut scored: HashMap<String, f32> = HashMap::new();
        let q = query.to_ascii_lowercase();

        // keyword branch
        for node in self.list_nodes(1000)? {
            let hay = format!("{} {}", node.kind, node.label).to_ascii_lowercase();
            if hay.contains(&q) {
                *scored.entry(node.id.clone()).or_insert(0.0) += 1.0 + node.importance;
            }
        }

        // vector branch
        if let Some(prov) = embedder {
            if let Ok(v) = prov.embeddings(vec![query.to_string()]).await {
                if let Some(qv) = v.into_iter().next() {
                    for vr in self.all_vectors()? {
                        let deq = dequant(&vr.quant, vr.scale);
                        let s = vector::cosine(&qv, &deq);
                        if s > 0.0 {
                            *scored.entry(vr.id.clone()).or_insert(0.0) += s;
                        }
                    }
                }
            }
        }

        let mut ids: Vec<(String, f32)> = scored.into_iter().collect();
        ids.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ids.truncate(k);

        let mut out = String::new();
        for (id, _score) in &ids {
            if let Some(n) = self.get_node(id)? {
                out.push_str(&format!("[{}] {} (importance {:.1})\n", n.kind, n.label, n.importance));
                for e in self.edges_for(&n.id)? {
                    out.push_str(&format!("    -({})-> {}\n", e.relation, e.target));
                }
            }
        }
        let facts = self.all_kv()?;
        for row in facts.iter().take(10) {
            out.push_str(&format!("[fact] {} = {}\n", row.key, row.value));
        }
        Ok(out)
    }
}
