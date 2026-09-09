//! Tool registry (OpenCode-style).
//!
//! AETHER's `Tool` trait is the per-tool contract; the registry is the
//! collection that hands tools to the agent loop. It is keyed by tool
//! `name()`. Toolsets are named groups of tool names used to scope the
//! surface per LLM role (planner / executor / reviewer).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use crate::Tool;

#[derive(Default)]
pub struct ToolRegistry {
    by_name: BTreeMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self { Self::default() }

    /// Register a tool. If a tool with the same name exists, it is replaced.
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        self.by_name.insert(tool.name().to_string(), tool);
    }

    pub fn unregister(&mut self, name: &str) -> Option<Arc<dyn Tool>> {
        self.by_name.remove(name)
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> { self.by_name.get(name).cloned() }
    pub fn names(&self) -> Vec<String> { self.by_name.keys().cloned().collect() }
    pub fn len(&self) -> usize { self.by_name.len() }
    pub fn is_empty(&self) -> bool { self.by_name.is_empty() }

    /// Resolve the tool surface for a toolset: returns the tool impls in
    /// the given role's `Toolset`. Unknown tool names are skipped (logged
    /// at warning level).
    pub fn for_toolset(&self, set: &Toolset) -> Vec<Arc<dyn Tool>> {
        let mut out = Vec::new();
        for name in &set.tools {
            if let Some(t) = self.by_name.get(name) {
                out.push(t.clone());
            } else {
                eprintln!("[tool-registry] toolset '{}' references unknown tool '{}'", set.name, name);
            }
        }
        out
    }
}

/// A named group of tool names. AETHER's three-LLM roles use distinct toolsets
/// so a planner cannot accidentally call file-write, etc.
#[derive(Debug, Clone, Default)]
pub struct Toolset {
    pub name: String,
    pub tools: Vec<String>,
}

impl Toolset {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), ..Default::default() }
    }
    pub fn with(mut self, tool: impl Into<String>) -> Self {
        self.tools.push(tool.into());
        self
    }
}

/// Builder for the canonical AETHER toolsets: planner, executor, reviewer.
pub fn canonical_toolsets() -> HashMap<&'static str, Toolset> {
    let mut out: HashMap<&'static str, Toolset> = HashMap::new();
    out.insert(
        "planner",
        Toolset::new("planner")
            .with("read_file")
            .with("list_directory")
            .with("grep")
            .with("git_status")
            .with("git_diff")
            .with("git_log"),
    );
    out.insert(
        "executor",
        Toolset::new("executor")
            .with("read_file")
            .with("write_file")
            .with("replace_in_file")
            .with("list_directory")
            .with("grep")
            .with("execute_command")
            .with("git_status")
            .with("git_diff")
            .with("git_log")
            .with("git_add")
            .with("git_commit"),
    );
    out.insert(
        "reviewer",
        Toolset::new("reviewer")
            .with("read_file")
            .with("list_directory")
            .with("grep")
            .with("git_diff")
            .with("git_log"),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use aether_models::Message;
    use aether_permissions::Permission;
    use serde_json::Value;
    use std::path::PathBuf;

    struct TestTool(&'static str);
    #[async_trait]
    impl Tool for TestTool {
        fn name(&self) -> &str { self.0 }
        fn description(&self) -> &str { "test" }
        fn category(&self) -> &'static str { "read" }
        fn required_permission(&self) -> Permission { Permission::Allow }
        fn json_schema(&self) -> Value { Value::Null }
        async fn execute(&self, _args: Value, _ctx: &crate::ToolContext) -> Result<crate::ToolResult, crate::ToolError> {
            Ok(crate::ToolResult { output: String::new(), is_error: false })
        }
    }

    #[test]
    fn register_and_lookup() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(TestTool("read_file")));
        assert!(r.get("read_file").is_some());
        assert!(r.get("missing").is_none());
    }

    #[test]
    fn replace_by_name() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(TestTool("read_file")));
        r.register(Arc::new(TestTool("read_file")));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn for_toolset_resolves_subset() {
        let mut r = ToolRegistry::new();
        r.register(Arc::new(TestTool("read_file")));
        r.register(Arc::new(TestTool("grep")));
        let set = Toolset::new("t").with("read_file").with("missing").with("grep");
        let got = r.for_toolset(&set);
        assert_eq!(got.len(), 2, "missing tool should be skipped");
        let names: Vec<String> = got.iter().map(|t| t.name().to_string()).collect();
        assert!(names.contains(&"read_file".to_string()));
        assert!(names.contains(&"grep".to_string()));
    }

    #[test]
    fn canonical_toolsets_resolve() {
        let r = ToolRegistry::new();
        let sets = canonical_toolsets();
        for (role, set) in &sets {
            let _ = (r.for_toolset(set), role);
        }
    }

    // silence unused import warnings in tests
    #[allow(dead_code)]
    fn _u() { let _ = (Message::default(), PathBuf::new()); }
}
