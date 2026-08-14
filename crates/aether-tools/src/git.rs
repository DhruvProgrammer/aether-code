//! Git tools (spec §5, Phase 2). Shell out to the `git` binary for parity with the
//! user's git config/hooks. Never auto-commit unless configured.

use super::{Tool, ToolContext, ToolError, ToolResult};
use aether_permissions::Permission;
use serde_json::Value;
use tokio::process::Command;

async fn run_git(ctx: &ToolContext, args: &[&str]) -> Result<ToolResult, ToolError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(&ctx.cwd)
        .output()
        .await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let text = format!(
        "exit={}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status.code().unwrap_or(-1),
        stdout,
        stderr
    );
    Ok(ToolResult { output: text, is_error: !output.status.success() })
}

macro_rules! git_tool {
    ($t:ident, $name:literal, $desc:literal, $cat:literal, $schema:expr, $body:expr) => {
        pub struct $t;
        #[async_trait::async_trait]
        impl Tool for $t {
            fn name(&self) -> &str { $name }
            fn description(&self) -> &str { $desc }
            fn category(&self) -> &'static str { $cat }
            fn required_permission(&self) -> Permission { Permission::Allow }
            fn json_schema(&self) -> Value { $schema }
            async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
                let build: fn(&Value) -> Vec<String> = $body;
                let args: Vec<String> = build(&args);
                let borrowed: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                run_git(ctx, &borrowed).await
            }
        }
    };
}

git_tool!(
    GitStatusTool,
    "git_status",
    "Show working tree status.",
    "read",
    serde_json::json!({ "type": "object", "properties": {}, "required": [] }),
    |_args| vec!["status".into()]
);

git_tool!(
    GitDiffTool,
    "git_diff",
    "Show unstaged/staged diff; optional path.",
    "read",
    serde_json::json!({ "type": "object", "properties": { "path": { "type": "string" } }, "required": [] }),
    |args| {
        let mut v = vec!["diff".to_string()];
        if let Some(p) = args.get("path").and_then(|x| x.as_str()) {
            v.push(p.to_string());
        }
        v
    }
);

git_tool!(
    GitLogTool,
    "git_log",
    "Show commit log; optional max count.",
    "read",
    serde_json::json!({ "type": "object", "properties": { "max": { "type": "integer" } }, "required": [] }),
    |args| {
        let mut v = vec!["log".to_string()];
        if let Some(n) = args.get("max").and_then(|x| x.as_u64()) {
            v.push(format!("-n{n}"));
        }
        v
    }
);

git_tool!(
    GitBranchTool,
    "git_branch",
    "List branches.",
    "read",
    serde_json::json!({ "type": "object", "properties": {}, "required": [] }),
    |_args| vec!["branch".into()]
);

git_tool!(
    GitCheckoutTool,
    "git_checkout",
    "Switch branches.",
    "edit",
    serde_json::json!({ "type": "object", "properties": { "branch": { "type": "string" } }, "required": ["branch"] }),
    |args| {
        let b = args.get("branch").and_then(|x| x.as_str()).unwrap_or("").to_string();
        vec!["checkout".into(), b]
    }
);

git_tool!(
    GitAddTool,
    "git_add",
    "Stage a file or path.",
    "git_commit",
    serde_json::json!({ "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] }),
    |args| {
        let p = args.get("path").and_then(|x| x.as_str()).unwrap_or(".").to_string();
        vec!["add".into(), p]
    }
);

git_tool!(
    GitCommitTool,
    "git_commit",
    "Create a commit with a message.",
    "git_commit",
    serde_json::json!({ "type": "object", "properties": { "message": { "type": "string" } }, "required": ["message"] }),
    |args| {
        let m = args.get("message").and_then(|x| x.as_str()).unwrap_or("").to_string();
        vec!["commit".into(), "-m".into(), m]
    }
);

/// All git tools for the default registry.
pub fn git_tools() -> Vec<std::sync::Arc<dyn Tool>> {
    vec![
        std::sync::Arc::new(GitStatusTool),
        std::sync::Arc::new(GitDiffTool),
        std::sync::Arc::new(GitLogTool),
        std::sync::Arc::new(GitBranchTool),
        std::sync::Arc::new(GitCheckoutTool),
        std::sync::Arc::new(GitAddTool),
        std::sync::Arc::new(GitCommitTool),
    ]
}
