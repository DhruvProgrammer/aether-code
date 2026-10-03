//! Semantic chunking: conversation messages -> memory candidates (§15).
//!
//! Boundaries follow engineering meaning (task transitions, corrections,
//! decisions, error/fix cycles, verification), never fixed message counts.
//! Token count is only a safety cap. Trivia is dropped (§58), secrets are
//! redacted before anything is stored (§60), and every chunk is
//! contextualized so it reads standalone (§23).

use crate::types::{MemoryRecord, MemoryType};

/// One observed conversation event. Built by the runtime from session
/// messages / tool calls — the raw record stays in `aether-sessions`.
#[derive(Debug, Clone)]
pub struct MessageView {
    pub role: String,
    pub content: String,
    pub tool_name: Option<String>,
    pub source_id: String,
}

/// Scope attached to everything extracted in one pass.
#[derive(Debug, Clone)]
pub struct ExtractContext {
    pub project_id: String,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    /// Short task label prepended to decontextualized chunks (§23).
    pub task_label: Option<String>,
}

fn is_trivial(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    if t.len() < 12 {
        return true;
    }
    matches!(
        t.as_str(),
        "ok" | "okay" | "thanks" | "thank you" | "sure" | "hello" | "hi"
            | "yes" | "no" | "got it" | "sounds good" | "lgtm"
    )
}

/// Redact likely secrets before storage (§60). Conservative patterns only.
pub fn redact_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let low = line.to_lowercase();
        let looks_secret = low.contains("sk-")
            || ((low.contains("password") || low.contains("passwd"))
                && (low.contains('=') || low.contains(':')))
            || ((low.contains("api_key") || low.contains("api-key") || low.contains("apikey"))
                && (low.contains('=') || low.contains(':')))
            || ((low.contains("token") || low.contains("secret"))
                && (low.contains('=') || low.contains(':'))
                && !low.contains("token budget")
                && !low.contains("tokens"));
        if looks_secret {
            // Keep the key name (useful context), drop the value.
            let cut = line.find(['=', ':']).map(|i| i + 1).unwrap_or(line.len());
            out.push_str(line[..cut].trim_end());
            out.push_str(" [REDACTED]\n");
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    // Bearer-style inline keys: `sk-AbC123...`.
    let mut clean = out;
    loop {
        let Some(i) = clean.to_lowercase().find("sk-") else {
            break;
        };
        let end = clean[i..]
            .find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
            .map(|e| i + e)
            .unwrap_or(clean.len());
        if end - i > 6 {
            clean.replace_range(i..end, "[REDACTED]");
        } else {
            break;
        }
    }
    clean.trim_end().to_string()
}

/// File paths mentioned in text (`src/x.rs`, `crates/a/b.toml`, ...).
pub fn extract_files(text: &str) -> Vec<String> {
    let mut files = Vec::new();
    for tok in text.split_whitespace() {
        let t = tok.trim_matches(|c: char| ".,;:!?()[]{}\"'`".contains(c));
        if t.len() < 4 || files.len() >= 8 {
            continue;
        }
        let has_sep = t.contains('/');
        let has_ext = t.rsplit('.').next().is_some_and(|e| {
            matches!(
                e,
                "rs" | "toml" | "ts" | "tsx" | "js" | "py" | "md" | "json"
                    | "yaml" | "yml" | "css" | "html" | "sh"
            )
        });
        if (has_sep && t.contains('.')) || (has_ext && (has_sep || t.len() < 64)) {
            if !files.contains(&t.to_string()) {
                files.push(t.to_string());
            }
        }
    }
    files
}

/// `Type::method` / `CamelCase` / `snake_case()` code symbols.
pub fn extract_symbols(text: &str) -> Vec<String> {
    let mut syms = Vec::new();
    for tok in text.split_whitespace() {
        let t = tok.trim_matches(|c: char| ".,;:!?()[]{}\"'`".contains(c));
        if t.len() < 3 || t.len() > 80 || syms.len() >= 12 {
            continue;
        }
        let scoped = t.contains("::");
        let camel = t.chars().next().is_some_and(|c| c.is_uppercase())
            && t.chars().any(|c| c.is_lowercase());
        if scoped || camel {
            if !syms.contains(&t.to_string()) {
                syms.push(t.to_string());
            }
        }
    }
    syms
}

fn contains_any(hay: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| hay.contains(n))
}

/// Classify one message into (type, importance, confidence).
fn classify(role: &str, tool: Option<&str>, low: &str) -> Option<(MemoryType, f32, f32)> {
    // User corrections outrank everything (§13).
    if role == "user"
        && contains_any(
            low,
            &[
                "don't",
                "do not",
                "wrong",
                "already told",
                "never ",
                "always ",
                "instead",
                "not that",
                "stop ",
            ],
        )
    {
        return Some((MemoryType::UserCorrection, 0.95, 0.9));
    }
    if contains_any(low, &["test passed", "tests passed", "build passed", "build succeeded"])
        || (contains_any(low, &["passed", "failed"])
            && contains_any(low, &["test", "build", "clippy", "check", "suite"]))
    {
        // Verified counts (e.g. "27 passed") outrank bare claims (§30).
        let evidenced = low.chars().any(|c| c.is_ascii_digit()) && low.contains("passed");
        return Some((MemoryType::Verification, 0.75, if evidenced { 0.85 } else { 0.6 }));
    }
    if contains_any(
        low,
        &["error", "failed", "failure", "panic", "traceback", "e0", "e1", "e2", "e3"],
    ) || tool.is_some_and(|t| t.contains("test"))
        && low.contains("fail")
    {
        return Some((MemoryType::Error, 0.7, 0.75));
    }
    if contains_any(low, &["fixed", "fix:", "fixes ", "resolved the"]) {
        return Some((MemoryType::Fix, 0.7, 0.7));
    }
    if contains_any(
        low,
        &["decided", "decision", "chose ", "will use", "going with", "agreed"],
    ) {
        return Some((MemoryType::Decision, 0.8, 0.7));
    }
    if role == "user"
        && (contains_any(
            low,
            &["implement", "add ", "create ", "must ", "should ", "requirement", "feature"],
        ) || low.len() > 60)
    {
        return Some((MemoryType::Requirement, 0.7, 0.6));
    }
    if contains_any(low, &["depends on", "dependency", "requires crate", "needs "])
        && contains_any(low, &["crate", "package", "library", "depends", "requires"])
    {
        return Some((MemoryType::Dependency, 0.6, 0.6));
    }
    if role == "assistant" && (!extract_files(low).is_empty() || low.contains("src/")) {
        return Some((MemoryType::FileInsight, 0.6, 0.6));
    }
    if tool.is_some() {
        return Some((MemoryType::ToolResult, 0.45, 0.8));
    }
    None
}

/// Turn observed messages into contextualized memory candidates.
/// Token count is a safety cap only — boundaries are semantic (§15).
pub fn extract_candidates(msgs: &[MessageView], ctx: &ExtractContext) -> Vec<MemoryRecord> {
    const MAX_CHUNK_CHARS: usize = 4000;
    let mut out = Vec::new();
    for m in msgs {
        let clean = redact_secrets(&m.content);
        if is_trivial(&clean) {
            continue;
        }
        let low = clean.to_lowercase();
        let Some((mtype, importance, confidence)) =
            classify(&m.role, m.tool_name.as_deref(), &low)
        else {
            continue;
        };
        let body: String = clean.chars().take(MAX_CHUNK_CHARS).collect();
        // Contextualize: a retrieved chunk must read standalone (§23).
        let mut content = String::new();
        if let Some(task) = &ctx.task_label {
            content.push_str(&format!("[task: {task}]\n"));
        }
        let files = extract_files(&body);
        if !files.is_empty() {
            content.push_str(&format!("[files: {}]\n", files.join(", ")));
        }
        content.push_str(&body);
        let first_line = body.lines().next().unwrap_or("").trim();
        let title: String = first_line.chars().take(120).collect();

        let mut rec = MemoryRecord::new(&ctx.project_id, mtype, title, content);
        rec.session_id = ctx.session_id.clone();
        rec.task_id = ctx.task_id.clone();
        rec.source_ids = vec![m.source_id.clone()];
        rec.files = files;
        rec.symbols = extract_symbols(&body);
        rec.importance = importance;
        rec.confidence = confidence;
        rec.tags = keyword_tags(&low);
        out.push(rec);
    }
    out
}

fn keyword_tags(low: &str) -> Vec<String> {
    let mut tags = Vec::new();
    for (kw, tag) in [
        ("provider", "provider"),
        ("auth", "auth"),
        ("test", "testing"),
        ("build", "build"),
        ("memory", "memory"),
        ("context", "context"),
        ("permission", "permissions"),
        ("tool", "tools"),
        ("session", "session"),
        ("compaction", "compaction"),
    ] {
        if low.contains(kw) && tags.len() < 6 {
            tags.push(tag.to_string());
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ExtractContext {
        ExtractContext {
            project_id: "p".into(),
            session_id: Some("s".into()),
            task_id: Some("t".into()),
            task_label: Some("provider validation".into()),
        }
    }

    fn msg(role: &str, content: &str) -> MessageView {
        MessageView {
            role: role.into(),
            content: content.into(),
            tool_name: None,
            source_id: "m1".into(),
        }
    }

    #[test]
    fn trivia_is_dropped() {
        let out = extract_candidates(&[msg("user", "ok"), msg("user", "thanks!")], &ctx());
        assert!(out.is_empty());
    }

    #[test]
    fn correction_is_high_priority_and_contextualized() {
        let out = extract_candidates(
            &[msg("user", "No, don't use Postgres. I already told you this project uses SQLite.")],
            &ctx(),
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].mem_type, MemoryType::UserCorrection);
        assert!((out[0].importance - 0.95).abs() < f32::EPSILON);
        assert!(out[0].content.contains("[task: provider validation]"));
    }

    #[test]
    fn secrets_never_reach_records() {
        let out = extract_candidates(
            &[msg("assistant", "I decided to set api_key = sk-live-abc123 in config for the provider")],
            &ctx(),
        );
        assert_eq!(out.len(), 1);
        assert!(!out[0].content.contains("sk-live-abc123"));
        assert!(out[0].content.contains("[REDACTED]"));
    }

    #[test]
    fn verification_confidence_needs_evidence() {
        let bare = extract_candidates(&[msg("assistant", "I ran the tests and everything passed for the provider")], &ctx());
        let ev = extract_candidates(&[msg("assistant", "cargo test: 27 passed, 2 failed in provider validation")], &ctx());
        assert_eq!(bare[0].mem_type, MemoryType::Verification);
        assert!(ev[0].confidence > bare[0].confidence);
    }

    #[test]
    fn files_and_symbols_extracted() {
        let out = extract_candidates(
            &[msg("assistant", "I changed Provider::validate in src/providers/nvidia.rs to fix the bug")],
            &ctx(),
        );
        assert_eq!(out[0].mem_type, MemoryType::FileInsight);
        assert!(out[0].files.contains(&"src/providers/nvidia.rs".to_string()));
        assert!(out[0].symbols.contains(&"Provider::validate".to_string()));
    }
}
