//! Skill index loader: reads `skills/software-engineering/index.json`
//! and resolves section files (full + compact).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectionMeta {
    pub id: String,
    pub file: String,
    #[serde(default)]
    pub compact: Option<String>,
    pub description: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub always: bool,
    #[serde(default)]
    pub priority: u32,
    #[serde(default)]
    pub requires: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexFile {
    id: String,
    #[allow(dead_code)]
    version: Option<String>,
    #[allow(dead_code)]
    source: Option<String>,
    sections: Vec<SectionMeta>,
}

/// Loaded skill index with resolved base directory.
pub struct SkillIndex {
    pub id: String,
    pub base_dir: PathBuf,
    sections: Vec<SectionMeta>,
    by_id: HashMap<String, usize>,
}

impl SkillIndex {
    /// Load from a skill directory containing `index.json`.
    pub fn load(skill_dir: &Path) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(skill_dir.join("index.json"))?;
        let parsed: IndexFile = serde_json::from_str(&raw)?;
        let mut by_id = HashMap::new();
        for (i, s) in parsed.sections.iter().enumerate() {
            by_id.insert(s.id.clone(), i);
        }
        Ok(Self {
            id: parsed.id,
            base_dir: skill_dir.to_path_buf(),
            sections: parsed.sections,
            by_id,
        })
    }

    /// Locate the bundled `skills/software-engineering` dir by walking up
    /// from the current exe dir, then cwd. Returns None if not found.
    pub fn locate_bundled() -> Option<PathBuf> {
        let candidates: Vec<PathBuf> = std::iter::once(
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf())),
        )
        .flatten()
        .chain(std::env::current_dir().ok())
        .collect();
        for base in candidates {
            let mut p = base.clone();
            loop {
                let cand = p.join("skills").join("software-engineering").join("index.json");
                if cand.exists() {
                    return Some(p.join("skills").join("software-engineering"));
                }
                if !p.pop() {
                    break;
                }
            }
        }
        None
    }

    pub fn sections(&self) -> &[SectionMeta] {
        &self.sections
    }

    pub fn get(&self, id: &str) -> Option<&SectionMeta> {
        self.by_id.get(id).map(|&i| &self.sections[i])
    }

    /// Read a section's full text.
    pub fn read_full(&self, meta: &SectionMeta) -> anyhow::Result<String> {
        Ok(std::fs::read_to_string(self.base_dir.join(&meta.file))?)
    }

    /// Read a section's compact text, falling back to full if no compact exists.
    pub fn read_compact(&self, meta: &SectionMeta) -> anyhow::Result<String> {
        if let Some(c) = &meta.compact {
            let p = self.base_dir.join(c);
            if p.exists() {
                return Ok(std::fs::read_to_string(p)?);
            }
        }
        self.read_full(meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_index() -> SkillIndex {
        // Build from the real repo skill dir when available; otherwise skip.
        match SkillIndex::locate_bundled() {
            Some(dir) => SkillIndex::load(&dir).expect("index loads"),
            None => panic!("bundled skill index not found (run from repo root)"),
        }
    }

    #[test]
    fn index_loads_all_sections() {
        let idx = test_index();
        assert_eq!(idx.id, "software-engineering");
        assert!(idx.sections().len() >= 20, "expected 20+ sections");
        assert!(idx.get("core").unwrap().always);
        assert!(idx.get("testing").is_some());
    }

    #[test]
    fn compact_falls_back_to_full() {
        let idx = test_index();
        let meta = idx.get("core").unwrap();
        let text = idx.read_compact(meta).unwrap();
        assert!(!text.is_empty());
    }
}
