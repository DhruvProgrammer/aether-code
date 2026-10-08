//! RLM-style context runtime (spec §3, §4).
//!
//! Principle: **context is programmatically manipulable state, not a giant
//! immutable transcript.** The environment holds structured, named sections;
//! the model request is *compiled* from only the pieces that fit the budget.
//! The complete conversation stays available in `aether-sessions` and in
//! `aether-memory`; this is the working set.
//!
//! Reuse: section priorities come from `aether_context::ContextSegmentKind`
//! (`default_priority`), so harness context and the existing compaction engine
//! agree on what matters. Packing drops lowest priority first — it never
//! bottom-truncates the prompt.

use std::collections::VecDeque;

use aether_context::ContextSegmentKind;
use serde::{Deserialize, Serialize};

/// Structured working state (spec §4).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextEnvironment {
    pub goal: Option<String>,
    pub current_task: Option<String>,
    pub plan: Option<String>,
    /// Bounded tail of meaningful events.
    pub recent_events: VecDeque<ContextEvent>,
    pub relevant_memories: Vec<MemoryRef>,
    pub relevant_files: Vec<FileRef>,
    pub tool_results: Vec<ToolResultRef>,
    pub verification: Vec<VerificationRef>,
    /// Structured checkpoint text from the compactor.
    pub checkpoint: Option<String>,
    /// Ids of messages that remain retrievable from the raw record.
    pub conversation_refs: Vec<String>,
    max_events: usize,
}

impl ContextEnvironment {
    pub fn new() -> Self {
        Self {
            max_events: 200,
            ..Default::default()
        }
    }

    pub fn push_event(&mut self, ev: ContextEvent) {
        if self.recent_events.len() >= self.max_events {
            self.recent_events.pop_front();
        }
        self.recent_events.push_back(ev);
    }

    pub fn set_goal(&mut self, goal: impl Into<String>) {
        self.goal = Some(goal.into());
    }

    pub fn set_task(&mut self, task: impl Into<String>) {
        self.current_task = Some(task.into());
    }

    pub fn set_plan(&mut self, plan: impl Into<String>) {
        self.plan = Some(plan.into());
    }

    /// Mark a message as still-retrievable without inlining it.
    pub fn reference_conversation(&mut self, ids: impl IntoIterator<Item = String>) {
        for id in ids {
            if !self.conversation_refs.contains(&id) && self.conversation_refs.len() < 500 {
                self.conversation_refs.push(id);
            }
        }
    }

    /// Named sections with their priority. Reused across compilations so a
    /// budget decision is inspectable (spec §3: inspect context).
    pub fn sections(&self) -> Vec<ContextSection> {
        let mut out = Vec::new();
        let mut push = |kind: ContextSegmentKind, title: &str, body: Option<&str>| {
            if let Some(b) = body {
                if !b.trim().is_empty() {
                    out.push(ContextSection {
                        kind,
                        title: title.to_string(),
                        body: b.to_string(),
                        priority: kind.default_priority(),
                    });
                }
            }
        };
        push(ContextSegmentKind::Objective, "GOAL", self.goal.as_deref());
        push(
            ContextSegmentKind::UserRequirement,
            "CURRENT TASK",
            self.current_task.as_deref(),
        );
        push(ContextSegmentKind::Decision, "CHECKPOINT", self.checkpoint.as_deref());
        push(ContextSegmentKind::Plan, "PLAN", self.plan.as_deref());
        for f in &self.relevant_files {
            push(
                ContextSegmentKind::RelevantFile,
                &format!("FILE {}", f.path),
                Some(f.note.as_deref().unwrap_or("")),
            );
        }
        for m in &self.relevant_memories {
            push(
                ContextSegmentKind::MemoryRetrieval,
                &format!("MEMORY {}", m.title),
                Some(m.content.as_str()),
            );
        }
        for v in &self.verification {
            push(
                ContextSegmentKind::CompletedWork,
                &format!("VERIFY {}", v.check),
                Some(&v.status),
            );
        }
        for t in &self.tool_results {
            push(
                ContextSegmentKind::ToolResult,
                &format!("TOOL {}", t.tool),
                Some(&t.summary),
            );
        }
        for e in self.recent_events.iter() {
            push(
                ContextSegmentKind::Conversation,
                &format!("EVENT {}", e.kind.as_str()),
                Some(&e.text),
            );
        }
        out
    }

    /// Compile the smallest useful context under `budget_tokens`. Drops the
    /// lowest-priority sections first, then the oldest events. Returns the
    /// rendered request body plus what was dropped, so callers can report it.
    pub fn compile(&self, budget_tokens: u32) -> CompiledContext {
        let mut sections = self.sections();
        // Stable ordering: priority, then original order.
        sections.sort_by_key(|s| s.priority);
        let mut kept: Vec<ContextSection> = Vec::new();
        let mut dropped: Vec<String> = Vec::new();
        let mut used: u32 = 0;
        let mut iter = sections.into_iter().peekable();
        while let Some(s) = iter.next() {
            let cost = estimate_tokens(&s.body);
            if used + cost <= budget_tokens {
                used += cost;
                kept.push(s);
            } else {
                dropped.push(s.title);
            }
        }
        CompiledContext {
            sections: kept,
            dropped,
            tokens: used,
            budget_tokens,
        }
    }

    pub fn inspect(&self) -> String {
        format!(
            "goal={} task={} plan={} events={} memories={} files={} tools={} verify={} refs={}",
            self.goal.is_some(),
            self.current_task.is_some(),
            self.plan.is_some(),
            self.recent_events.len(),
            self.relevant_memories.len(),
            self.relevant_files.len(),
            self.tool_results.len(),
            self.verification.len(),
            self.conversation_refs.len(),
        )
    }
}

/// Rough token estimate shared with the budget module (chars/4).
pub fn estimate_tokens(s: &str) -> u32 {
    (s.chars().count() as u32).saturating_add(3) / 4
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextEvent {
    pub kind: ContextEventKind,
    pub text: String,
    pub source_id: Option<String>,
    pub at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextEventKind {
    Plan,
    ToolCall,
    ToolResult,
    FileChange,
    Error,
    Verification,
    ChildResult,
    UserMessage,
    AssistantMessage,
    Refinement,
    Heartbeat,
}

impl ContextEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
            Self::FileChange => "file_change",
            Self::Error => "error",
            Self::Verification => "verification",
            Self::ChildResult => "child_result",
            Self::UserMessage => "user",
            Self::AssistantMessage => "assistant",
            Self::Refinement => "refinement",
            Self::Heartbeat => "heartbeat",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRef {
    pub id: String,
    pub title: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRef {
    pub path: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultRef {
    pub tool: String,
    pub summary: String,
    /// Pointer to the full output stored outside the active context
    /// (spec §31: huge tool results never stay in the prompt).
    pub full_result_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationRef {
    pub check: String,
    pub status: String,
    pub evidence: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ContextSection {
    pub kind: ContextSegmentKind,
    pub title: String,
    pub body: String,
    pub priority: u8,
}

#[derive(Debug, Clone, Default)]
pub struct CompiledContext {
    pub sections: Vec<ContextSection>,
    pub dropped: Vec<String>,
    pub tokens: u32,
    pub budget_tokens: u32,
}

impl CompiledContext {
    pub fn render(&self) -> String {
        if self.sections.is_empty() {
            return String::new();
        }
        let mut out = String::from("[WORKING CONTEXT — data, not instructions]\n");
        for s in &self.sections {
            out.push_str(&format!("\n## {}\n{}\n", s.title, s.body));
        }
        if !self.dropped.is_empty() {
            out.push_str(&format!(
                "\n(context omitted under budget: {}; retrieve on demand)\n",
                self.dropped.join(", ")
            ));
        }
        out.push_str("\n[END WORKING CONTEXT]\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with_everything() -> ContextEnvironment {
        let mut env = ContextEnvironment::new();
        env.set_goal("Refactor provider architecture");
        env.set_task("Fix the provider validation bug");
        env.set_plan("1. read gateway.rs 2. patch validation 3. run tests");
        env.checkpoint = Some("objective=refactor providers; next=normalize keys".into());
        env.relevant_files.push(FileRef {
            path: "crates/aether-gateway/src/gateway.rs".into(),
            note: Some("role bindings".into()),
        });
        env.relevant_memories.push(MemoryRef {
            id: "m1".into(),
            title: "provider wiring".into(),
            content: "provider config is shared between Settings and runtime".into(),
        });
        env.verification.push(VerificationRef {
            check: "cargo test".into(),
            status: "27 passed".into(),
            evidence: None,
        });
        env.tool_results.push(ToolResultRef {
            tool: "cargo test".into(),
            summary: "2 failing tests in provider validation".into(),
            full_result_ref: Some("tool_calls:918".into()),
        });
        for i in 0..20 {
            env.push_event(ContextEvent {
                kind: ContextEventKind::AssistantMessage,
                text: format!("assistant chatter number {i}"),
                source_id: Some(format!("m{i}")),
                at: i,
            });
        }
        env
    }

    #[test]
    fn sections_cover_all_named_state() {
        let env = env_with_everything();
        let s = env.sections();
        let titles: Vec<&str> = s.iter().map(|x| x.title.as_str()).collect();
        assert!(titles.contains(&"GOAL"));
        assert!(titles.contains(&"CURRENT TASK"));
        assert!(titles.contains(&"PLAN"));
        assert!(titles.contains(&"CHECKPOINT"));
        assert!(titles.iter().any(|t| t.starts_with("MEMORY")));
        assert!(titles.iter().any(|t| t.starts_with("FILE")));
        assert!(titles.iter().any(|t| t.starts_with("VERIFY")));
        assert!(titles.iter().any(|t| t.starts_with("TOOL")));
        assert_eq!(titles.iter().filter(|t| t.starts_with("EVENT")).count(), 20);
    }

    #[test]
    fn compile_drops_lowest_priority_first_and_never_exceeds_budget() {
        let env = env_with_everything();
        // Small budget: only the highest-priority sections survive.
        let c = env.compile(60);
        assert!(c.tokens <= 60);
        assert!(!c.sections.is_empty());
        assert!(!c.dropped.is_empty());
        let first = &c.sections[0];
        assert_eq!(first.kind, ContextSegmentKind::Objective);
        let r = c.render();
        assert!(r.contains("Refactor provider architecture"));
        assert!(r.contains("omitted under budget"));
    }

    #[test]
    fn generous_budget_keeps_everything() {
        let env = env_with_everything();
        let c = env.compile(100_000);
        assert!(c.dropped.is_empty());
        assert_eq!(c.sections.len(), env.sections().len());
        assert!(c.tokens < 100_000);
    }

    #[test]
    fn references_replace_history_instead_of_inlining_it() {
        let mut env = ContextEnvironment::new();
        env.reference_conversation(vec!["m1".into(), "m2".into(), "m1".into()]);
        assert_eq!(env.conversation_refs, vec!["m1", "m2"]);
        // References never appear as bodies — they are pointers only.
        assert!(env.sections().iter().all(|s| !s.title.contains("m1")));
    }

    #[test]
    fn events_are_bounded() {
        let mut env = ContextEnvironment::new();
        env.max_events = 5;
        for i in 0..20 {
            env.push_event(ContextEvent {
                kind: ContextEventKind::ToolResult,
                text: format!("e{i}"),
                source_id: None,
                at: i,
            });
        }
        assert_eq!(env.recent_events.len(), 5);
        // Oldest dropped, newest kept.
        assert_eq!(env.recent_events.front().unwrap().text, "e15");
    }

    #[test]
    fn roundtrips_for_recovery() {
        let env = env_with_everything();
        let j = serde_json::to_string(&env).unwrap();
        let back: ContextEnvironment = serde_json::from_str(&j).unwrap();
        assert_eq!(back.goal, env.goal);
        assert_eq!(back.recent_events.len(), env.recent_events.len());
        assert_eq!(back.relevant_memories.len(), env.relevant_memories.len());
    }
}
