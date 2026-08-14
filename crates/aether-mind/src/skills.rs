//! Skills discovery (spec §10). Lazily indexes `SKILL.md` files by name + description
//! only — never loads skill bodies into context unless queried.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: String,
}

#[derive(Debug, Default)]
pub struct SkillIndex {
    skills: Vec<Skill>,
}

impl SkillIndex {
    /// Discover SKILL.md files under `root`, bounded by depth (spec §10: lazy, cheap).
    pub fn discover(root: &Path) -> Arc<Self> {
        let mut skills = Vec::new();
        walk(root, 0, 5, &mut skills);
        Arc::new(Self { skills })
    }

    pub fn all(&self) -> &[Skill] {
        &self.skills
    }

    /// Case-insensitive substring match over name + description.
    pub fn search(&self, query: &str) -> Vec<&Skill> {
        let q = query.to_ascii_lowercase();
        self.skills
            .iter()
            .filter(|s| {
                s.name.to_ascii_lowercase().contains(&q)
                    || s.description.to_ascii_lowercase().contains(&q)
            })
            .collect()
    }
}

fn walk(dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<Skill>) {
    if depth > max_depth {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if matches!(name, ".git" | "node_modules" | "target") {
                continue;
            }
        }
        if p.is_dir() {
            walk(&p, depth + 1, max_depth, out);
        } else if p.file_name().and_then(|n| n.to_str()) == Some("SKILL.md")
            || p.file_name().and_then(|n| n.to_str()) == Some("skill.md")
        {
            out.push(parse_skill(&p));
        }
    }
}

fn parse_skill(path: &Path) -> Skill {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut name = String::new();
    let mut description = String::new();
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("name:") {
            name = rest.trim().to_string();
        } else if let Some(rest) = l.strip_prefix("description:") {
            description = rest.trim().to_string();
        }
        if !name.is_empty() && !description.is_empty() {
            break;
        }
    }
    Skill {
        name: if name.is_empty() {
            path.file_name().and_then(|n| n.to_str()).unwrap_or("skill").to_string()
        } else {
            name
        },
        description,
        path: path.to_string_lossy().to_string(),
    }
}

/// Re-export for convenience.
pub type SkillPath = PathBuf;
