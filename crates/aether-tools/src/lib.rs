//! Tool system (spec §5). Every tool implements one trait so new tools plug in
//! without touching the agent core. Commands run via `tokio::process::Command`
//! (never shell-string concatenation from untrusted input).

use async_trait::async_trait;
use aether_permissions::Permission;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub mod analysis;
pub struct ToolContext {
    pub cwd: PathBuf,
}

#[derive(Debug)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("anyhow: {0}")]
    Anyhow(#[from] anyhow::Error),
    #[error("tool error: {0}")]
    Other(String),
}

/// Category maps a tool to a policy key (spec §14): read | edit | bash | delete |
/// git_commit | network. The agent consults `Policy::value_for(category)`.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn json_schema(&self) -> Value;
    /// Policy category this tool falls under.
    fn category(&self) -> &'static str;
    /// Intrinsic minimum permission the tool requires (combined with policy).
    fn required_permission(&self) -> Permission;
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError>;
}

fn arg_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// File tools
// ---------------------------------------------------------------------------

pub struct ReadFileTool;
#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str { "read_file" }
    fn description(&self) -> &str { "Read a UTF-8 file from disk." }
    fn category(&self) -> &'static str { "read" }
    fn required_permission(&self) -> Permission { Permission::Allow }
    fn json_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string", "description": "Path relative to cwd" } },
            "required": ["path"]
        })
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let path = arg_str(&args, "path").ok_or_else(|| ToolError::Other("missing 'path'".into()))?;
        let full = ctx.cwd.join(path);
        let content = std::fs::read_to_string(&full)?;
        Ok(ToolResult { output: content, is_error: false })
    }
}

pub struct WriteFileTool;
#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str { "write_file" }
    fn description(&self) -> &str { "Write content to a file, creating parent dirs." }
    fn category(&self) -> &'static str { "edit" }
    fn required_permission(&self) -> Permission { Permission::Allow }
    fn json_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string" }, "content": { "type": "string" } },
            "required": ["path", "content"]
        })
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let path = arg_str(&args, "path").ok_or_else(|| ToolError::Other("missing 'path'".into()))?;
        let content = arg_str(&args, "content").ok_or_else(|| ToolError::Other("missing 'content'".into()))?;
        let full = ctx.cwd.join(path);
        sandbox_check(&full, &ctx.cwd)?;
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&full, content)?;
        Ok(ToolResult { output: format!("wrote {}", full.display()), is_error: false })
    }
}

/// Normalize a canonicalized path by stripping the Windows extended-length `\\?\` prefix so
/// two paths produced by different canonicalize results can be compared consistently.
#[cfg(windows)]
fn strip_unc(p: &Path) -> std::path::PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        std::path::PathBuf::from(rest)
    } else {
        p.to_path_buf()
    }
}
#[cfg(not(windows))]
fn strip_unc(p: &Path) -> std::path::PathBuf { p.to_path_buf() }

/// Reject `full` if it does not lie inside `cwd` after canonicalization. Prevents sandbox
/// escapes such as `path = "../escape.txt"` or `path = "/etc/passwd"`.
fn sandbox_check(full: &Path, cwd: &Path) -> Result<(), ToolError> {
    let cwd_canon = strip_unc(&cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf()));
    let full_canon = strip_unc(&full.canonicalize().unwrap_or_else(|_| full.to_path_buf()));
    let cwd_str = cwd_canon.to_string_lossy().replace('\\', "/");
    let full_str = full_canon.to_string_lossy().replace('\\', "/");
    if full_str == cwd_str || full_str.starts_with(&format!("{cwd_str}/")) {
        Ok(())
    } else {
        Err(ToolError::Other(format!(
            "path '{}' is outside cwd '{}'",
            full.display(),
            cwd.display()
        )))
    }
}

pub struct ListDirectoryTool;
#[async_trait]
impl Tool for ListDirectoryTool {
    fn name(&self) -> &str { "list_directory" }
    fn description(&self) -> &str { "List entries in a directory." }
    fn category(&self) -> &'static str { "read" }
    fn required_permission(&self) -> Permission { Permission::Allow }
    fn json_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": []
        })
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let path = arg_str(&args, "path").unwrap_or_else(|| ".".into());
        let full = ctx.cwd.join(path);
        let mut out = String::new();
        for e in std::fs::read_dir(&full)? {
            let e = e?;
            let kind = if e.path().is_dir() { "dir  " } else { "file " };
            out.push_str(&format!("{}{}\n", kind, e.file_name().to_string_lossy()));
        }
        Ok(ToolResult { output: out, is_error: false })
    }
}

pub struct GrepTool;
#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str { "grep" }
    fn description(&self) -> &str { "Case-insensitive substring search across text files." }
    fn category(&self) -> &'static str { "read" }
    fn required_permission(&self) -> Permission { Permission::Allow }
    fn json_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": { "pattern": { "type": "string" }, "path": { "type": "string" } },
            "required": ["pattern"]
        })
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let pattern = arg_str(&args, "pattern").ok_or_else(|| ToolError::Other("missing 'pattern'".into()))?;
        let root = ctx.cwd.join(arg_str(&args, "path").unwrap_or_else(|| ".".into()));
        let mut out = String::new();
        grep_walk(&root, &pattern, &mut out, 0);
        Ok(ToolResult { output: out, is_error: false })
    }
}

fn grep_walk(root: &Path, pattern: &str, out: &mut String, depth: usize) {
    if depth > 8 {
        return;
    }
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    let pat = pattern.to_ascii_lowercase();
    for entry in entries.flatten() {
        let p = entry.path();
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if matches!(name, ".git" | "node_modules" | "target") {
                continue;
            }
        }
        if p.is_dir() {
            grep_walk(&p, pattern, out, depth + 1);
        } else if let Ok(content) = std::fs::read_to_string(&p) {
            for (i, line) in content.lines().enumerate() {
                if line.to_ascii_lowercase().contains(&pat) {
                    out.push_str(&format!("{}:{}: {}\n", p.display(), i + 1, line));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Terminal tool
// ---------------------------------------------------------------------------

pub struct ExecuteCommandTool;
#[async_trait]
impl Tool for ExecuteCommandTool {
    fn name(&self) -> &str { "execute_command" }
    fn description(&self) -> &str { "Run a shell command; captures stdout/stderr/exit/duration." }
    fn category(&self) -> &'static str { "bash" }
    fn required_permission(&self) -> Permission { Permission::Allow }
    fn json_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": { "command": { "type": "string" } },
            "required": ["command"]
        })
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let command = arg_str(&args, "command").ok_or_else(|| ToolError::Other("missing 'command'".into()))?;
        let (shell, flag) = if cfg!(target_os = "windows") {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };
        let start = std::time::Instant::now();
        let output = tokio::process::Command::new(shell)
            .arg(flag)
            .arg(&command)
            .current_dir(&ctx.cwd)
            .output()
            .await?;
        let dur = start.elapsed();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let text = format!(
            "exit={}\nduration_ms={}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            output.status.code().unwrap_or(-1),
            dur.as_millis(),
            stdout,
            stderr
        );
        Ok(ToolResult { output: text, is_error: !output.status.success() })
    }
}

// ---------------------------------------------------------------------------
// Git tools (spec §5 / Phase 2 — shell out to the git binary)
// ---------------------------------------------------------------------------

pub mod git;
pub mod mcp;
pub use git::*;

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Phase 2 default toolset (fs + terminal + git).
pub fn default_tools() -> Vec<Arc<dyn Tool>> {
    let mut v: Vec<Arc<dyn Tool>> = vec![
        Arc::new(ReadFileTool),
        Arc::new(WriteFileTool),
        Arc::new(ListDirectoryTool),
        Arc::new(GrepTool),
        Arc::new(ExecuteCommandTool),
    ];
    v.extend(git_tools());
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_cwd(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aether-tool-test-{tag}-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[tokio::test]
    async fn write_file_sandbox_rejects_path_traversal() {
        let cwd = tmp_cwd("sandbox");
        let ctx = ToolContext { cwd: cwd.clone() };
        // `..` escape must be rejected.
        let res = WriteFileTool
            .execute(serde_json::json!({ "path": "../escape.txt", "content": "no" }), &ctx)
            .await;
        assert!(res.is_err(), "write_file must reject paths outside cwd");
        // Absolute path must be rejected.
        let res = WriteFileTool
            .execute(serde_json::json!({ "path": "C:\\Windows\\System32\\evil.txt", "content": "no" }), &ctx)
            .await;
        assert!(res.is_err(), "write_file must reject absolute paths outside cwd");
    }

    #[tokio::test]
    async fn write_file_accepts_cwd_and_subdir() {
        let cwd = tmp_cwd("sandbox-ok");
        let ctx = ToolContext { cwd: cwd.clone() };
        let res = WriteFileTool
            .execute(serde_json::json!({ "path": "inside.txt", "content": "ok" }), &ctx)
            .await;
        assert!(res.is_ok(), "write_file within cwd should succeed: {:?}", res);
        let res = WriteFileTool
            .execute(serde_json::json!({ "path": "sub/inside.txt", "content": "ok" }), &ctx)
            .await;
        assert!(res.is_ok(), "write_file within cwd subdir should succeed: {:?}", res);
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[tokio::test]
    async fn read_file_reports_missing_path() {
        let cwd = tmp_cwd("read");
        let ctx = ToolContext { cwd };
        let res = ReadFileTool
            .execute(serde_json::json!({ "path": "does-not-exist.txt" }), &ctx)
            .await;
        assert!(res.is_err());
    }
}
