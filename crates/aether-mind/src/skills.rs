//! Skills System — on-demand capability/instruction packages for AETHER.
//!
//! Replaces the v0.11 "name + description + path" stub with a real skill
//! registry. Each skill is a self-describing Markdown file (SKILL.md or
//! skill.toml) that declares:
//!   * metadata (id, name, version, author, tags)
//!   * required permissions (forwarded to the PermissionEngine when loaded)
//!   * required tools (forwarded to the tool allowlist when loaded)
//!   * supported agents
//!   * the actual instructions / examples / dependencies / templates
//!
//! Skills are NOT auto-loaded. Agents `discover → evaluate → request/load`
//! skills explicitly. The loader emits an activation event the runtime can
//! use to surface it in the UI and feed it into the agent's context.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub author: String,
    pub tags: Vec<String>,
    /// Permissions the skill requires to run (forwarded to PermissionEngine).
    pub required_permissions: Vec<String>,
    /// Tool names the skill relies on.
    pub required_tools: Vec<String>,
    /// Agent ids allowed to load this skill.
    pub supported_agents: Vec<String>,
    /// Optional structured sections.
    pub instructions: String,
    pub examples: Vec<String>,
    pub workflows: Vec<SkillWorkflow>,
    pub templates: Vec<SkillTemplate>,
    pub validation_rules: Vec<String>,
    pub dependencies: Vec<String>,
    /// On-disk source.
    pub source_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillWorkflow {
    pub name: String,
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillTemplate {
    pub name: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub tags: Vec<String>,
    pub source_path: PathBuf,
}

impl From<&Skill> for SkillSummary {
    fn from(s: &Skill) -> Self {
        Self {
            id: s.id.clone(),
            name: s.name.clone(),
            description: s.description.clone(),
            version: s.version.clone(),
            tags: s.tags.clone(),
            source_path: s.source_path.clone(),
        }
    }
}

/// Discovers and indexes skills, plus a method to load full bodies.
#[derive(Debug, Default)]
pub struct SkillRegistry {
    skills: Vec<Skill>,
}

impl SkillRegistry {
    pub fn new() -> Self { Self::default() }

    /// Walk a root directory up to `max_depth` and index any `SKILL.md` /
    /// `skill.md` / `skill.toml` files found.
    pub fn discover(root: &Path) -> Arc<Self> {
        let mut r = Self::default();
        let _ = r.scan_more(root, 5);
        Arc::new(r)
    }

    /// Like [`discover`] but also registers the compile-time bundled skills
    /// (e.g. `sonarqube-analysis`) so they work even in projects without a
    /// local skills directory.
    pub fn discover_with_bundled(root: &Path) -> Arc<Self> {
        let mut r = Self::default();
        let _ = r.scan_more(root, 5);
        r.register_bundled();
        Arc::new(r)
    }

    /// Walk a root directory up to `max_depth` and index any `SKILL.md` /
    /// `skill.md` / `skill.toml` files found. Public so the desktop app can
    /// scan additional roots (e.g. `~/.aether/skills` and the bundled
    /// resources directory).
    pub fn scan_more(&mut self, root: &Path, max_depth: usize) -> std::io::Result<()> {
        walk(root, 0, max_depth, &mut self.skills);
        Ok(())
    }

    /// Add a skill manually (used by plugins).
    pub fn register(&mut self, skill: Skill) {
        if !self.skills.iter().any(|s| s.id == skill.id) {
            self.skills.push(skill);
        }
    }

    /// Register all bundled (compile-time embedded) skills. Idempotent.
    pub fn register_bundled(&mut self) {
        for s in bundled_skills() {
            self.register(s);
        }
    }

    pub fn all(&self) -> &[Skill] { &self.skills }
    pub fn summaries(&self) -> Vec<SkillSummary> { self.skills.iter().map(SkillSummary::from).collect() }

    pub fn get(&self, id: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.id == id)
    }

    pub fn search(&self, query: &str) -> Vec<&Skill> {
        let q = query.to_ascii_lowercase();
        self.skills
            .iter()
            .filter(|s| {
                s.name.to_ascii_lowercase().contains(&q)
                    || s.id.to_ascii_lowercase().contains(&q)
                    || s.description.to_ascii_lowercase().contains(&q)
                    || s.tags.iter().any(|t| t.to_ascii_lowercase().contains(&q))
            })
            .collect()
    }

    /// Skills that match every requested tag (intersection).
    pub fn filter_by_tags(&self, tags: &[String]) -> Vec<&Skill> {
        let want: HashSet<&str> = tags.iter().map(|t| t.as_str()).collect();
        self.skills
            .iter()
            .filter(|s| want.iter().all(|t| s.tags.iter().any(|x| x == t)))
            .collect()
    }

    /// Skills a given agent id is allowed to load.
    pub fn for_agent(&self, agent_id: &str) -> Vec<&Skill> {
        self.skills
            .iter()
            .filter(|s| s.supported_agents.is_empty() || s.supported_agents.iter().any(|a| a == agent_id))
            .collect()
    }

    /// Load a skill by id. Returns the full skill body (for context injection).
    pub fn load(&self, id: &str) -> Option<&Skill> { self.get(id) }

    /// Compose multiple skills into a single "skill bundle" body for
    /// injection. Returns the merged instructions + a list of contributing
    /// skill ids.
    pub fn compose(&self, ids: &[String]) -> Option<SkillBundle> {
        let mut bundle = SkillBundle { ids: Vec::new(), instructions: String::new(), examples: Vec::new() };
        for id in ids {
            if let Some(s) = self.get(id) {
                bundle.ids.push(s.id.clone());
                bundle.instructions.push_str("\n\n## Skill: ");
                bundle.instructions.push_str(&s.name);
                bundle.instructions.push_str("\n\n");
                bundle.instructions.push_str(&s.instructions);
                bundle.examples.extend(s.examples.iter().cloned());
            }
        }
        if bundle.ids.is_empty() { None } else { Some(bundle) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillBundle {
    pub ids: Vec<String>,
    pub instructions: String,
    pub examples: Vec<String>,
}

fn walk(dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<Skill>) {
    if depth > max_depth { return; }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if matches!(name, ".git" | "node_modules" | "target") { continue; }
        }
        if p.is_dir() {
            walk(&p, depth + 1, max_depth, out);
        } else if let Some(fname) = p.file_name().and_then(|n| n.to_str()) {
            if matches!(fname, "SKILL.md" | "skill.md") {
                if let Some(s) = parse_markdown_skill(&p) { out.push(s); }
            } else if fname == "skill.toml" {
                if let Some(s) = parse_toml_skill(&p) { out.push(s); }
            }
        }
    }
}

fn parse_markdown_skill(path: &Path) -> Option<Skill> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut name = String::new();
    let mut description = String::new();
    let mut version = "0.0.0".into();
    let mut author = String::new();
    let mut tags: Vec<String> = Vec::new();
    let mut required_permissions: Vec<String> = Vec::new();
    let mut required_tools: Vec<String> = Vec::new();
    let mut supported_agents: Vec<String> = Vec::new();

    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("name:")        { name = rest.trim().into(); }
        else if let Some(rest) = l.strip_prefix("description:") { description = rest.trim().into(); }
        else if let Some(rest) = l.strip_prefix("version:")     { version = rest.trim().into(); }
        else if let Some(rest) = l.strip_prefix("author:")      { author = rest.trim().into(); }
        else if let Some(rest) = l.strip_prefix("tags:")        { tags = rest.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(); }
        else if let Some(rest) = l.strip_prefix("required_permissions:") { required_permissions = rest.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(); }
        else if let Some(rest) = l.strip_prefix("required_tools:")       { required_tools = rest.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(); }
        else if let Some(rest) = l.strip_prefix("supported_agents:")     { supported_agents = rest.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(); }
    }
    let id = path
        .parent().and_then(|p| p.file_name()).and_then(|n| n.to_str())
        .unwrap_or("skill").to_string();
    if name.is_empty() { name = id.clone(); }
    Some(Skill {
        id,
        name,
        description,
        version,
        author,
        tags,
        required_permissions,
        required_tools,
        supported_agents,
        instructions: text,
        examples: Vec::new(),
        workflows: Vec::new(),
        templates: Vec::new(),
        validation_rules: Vec::new(),
        dependencies: Vec::new(),
        source_path: path.to_path_buf(),
    })
}

fn parse_toml_skill(path: &Path) -> Option<Skill> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut s: Skill = toml::from_str(&text).ok()?;
    s.source_path = path.to_path_buf();
    Some(s)
}

/// Backward-compat alias kept so existing `SkillIndex` callers compile.
pub use SkillRegistry as SkillIndex;
pub use SkillSummary as SkillPathSummary;

/// Skills embedded at compile time so they are available even when the user's
/// workspace has no `.aether/skills`. They still obey the on-demand loading
/// philosophy — registration just makes them discoverable.
pub fn bundled_skills() -> Vec<Skill> {
    let mut out = Vec::new();
    if let Some(s) = parse_markdown_skill_text(
        "sonarqube-analysis",
        include_str!("../skills/sonarqube-analysis/SKILL.md"),
    ) {
        out.push(s);
    }
    out
}

/// Parse a SKILL.md body from a string with an explicit skill id.
fn parse_markdown_skill_text(id: &str, text: &str) -> Option<Skill> {
    let mut name = String::new();
    let mut description = String::new();
    let mut version = "0.0.0".into();
    let mut author = String::new();
    let mut tags: Vec<String> = Vec::new();
    let mut required_permissions: Vec<String> = Vec::new();
    let mut required_tools: Vec<String> = Vec::new();
    let mut supported_agents: Vec<String> = Vec::new();

    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("name:")        { name = rest.trim().into(); }
        else if let Some(rest) = l.strip_prefix("description:") { description = rest.trim().into(); }
        else if let Some(rest) = l.strip_prefix("version:")     { version = rest.trim().into(); }
        else if let Some(rest) = l.strip_prefix("author:")      { author = rest.trim().into(); }
        else if let Some(rest) = l.strip_prefix("tags:")        { tags = rest.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(); }
        else if let Some(rest) = l.strip_prefix("required_permissions:") { required_permissions = rest.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(); }
        else if let Some(rest) = l.strip_prefix("required_tools:")       { required_tools = rest.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(); }
        else if let Some(rest) = l.strip_prefix("supported_agents:")     { supported_agents = rest.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(); }
    }
    if name.is_empty() { name = id.to_string(); }
    Some(Skill {
        id: id.to_string(),
        name,
        description,
        version,
        author,
        tags,
        required_permissions,
        required_tools,
        supported_agents,
        instructions: text.to_string(),
        examples: Vec::new(),
        workflows: Vec::new(),
        templates: Vec::new(),
        validation_rules: Vec::new(),
        dependencies: Vec::new(),
        source_path: PathBuf::from(format!("bundled:{id}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_temp_skill(dir: &Path, id: &str, name: &str, desc: &str, tags: &[&str]) {
        let sub = dir.join(id);
        std::fs::create_dir_all(&sub).unwrap();
        let mut f = std::fs::File::create(sub.join("SKILL.md")).unwrap();
        writeln!(f, "name: {}", name).unwrap();
        writeln!(f, "description: {}", desc).unwrap();
        writeln!(f, "tags: {}", tags.join(",")).unwrap();
        writeln!(f, "\n# Body\n\nDo the thing.").unwrap();
    }

    #[test]
    fn discover_indexes_markdown() {
        let tmp = tempdir();
        make_temp_skill(&tmp, "git", "Git Workflows", "git-related guidance", &["git", "vcs"]);
        make_temp_skill(&tmp, "frontend", "Frontend Patterns", "react + ts", &["frontend"]);
        let r = SkillRegistry::discover(&tmp);
        assert_eq!(r.all().len(), 2);
        assert!(r.get("git").is_some());
        assert!(r.get("frontend").is_some());
    }

    #[test]
    fn search_finds_by_tag() {
        let tmp = tempdir();
        make_temp_skill(&tmp, "git", "Git", "git stuff", &["git"]);
        make_temp_skill(&tmp, "frontend", "FE", "fe stuff", &["frontend"]);
        let r = SkillRegistry::discover(&tmp);
        let hits = r.search("git");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "git");
    }

    #[test]
    fn filter_by_tags_intersects() {
        let tmp = tempdir();
        make_temp_skill(&tmp, "git", "Git", "d", &["git", "vcs"]);
        let r = SkillRegistry::discover(&tmp);
        let hits = r.filter_by_tags(&vec!["git".into(), "vcs".into()]);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn compose_merges_bodies() {
        let tmp = tempdir();
        make_temp_skill(&tmp, "a", "A", "d", &["x"]);
        make_temp_skill(&tmp, "b", "B", "d", &["y"]);
        let r = SkillRegistry::discover(&tmp);
        let bundle = r.compose(&vec!["a".into(), "b".into()]).unwrap();
        assert_eq!(bundle.ids.len(), 2);
        assert!(bundle.instructions.contains("Skill: A"));
        assert!(bundle.instructions.contains("Skill: B"));
    }

    #[test]
    fn bundled_sonarqube_skill_is_registered() {
        let tmp = tempdir();
        let r = SkillRegistry::discover_with_bundled(&tmp);
        let s = r.get("sonarqube-analysis").expect("bundled sonarqube skill");
        assert!(s.required_tools.contains(&"analyze_code".to_string()));
        assert!(s.required_tools.contains(&"analysis_status".to_string()));
        assert!(s.tags.contains(&"sonarqube".to_string()));
        assert!(s.instructions.contains("Authority chain"));
        // controller may load it; a random agent not listed may not.
        assert!(r.for_agent("controller").iter().any(|x| x.id == "sonarqube-analysis"));
        assert!(!r.for_agent("nonexistent-agent").iter().any(|x| x.id == "sonarqube-analysis"));
    }

    #[test]
    fn bundled_registration_is_idempotent() {
        let mut r = SkillRegistry::new();
        r.register_bundled();
        r.register_bundled();
        let count = r.all().iter().filter(|s| s.id == "sonarqube-analysis").count();
        assert_eq!(count, 1);
    }

    fn tempdir() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("aether-skills-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
