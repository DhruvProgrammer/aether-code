//! Prompt compiler: structured components → budgeted model request.
//!
//! Fixed ordering (highest priority first):
//! SYSTEM → ROLE → SKILLS → WORKSPACE → MEMORY → REFERENCES → CHECKPOINT →
//! RECENT → REQUEST. Retrieved content is DATA, never instructions.

use serde::{Deserialize, Serialize};

use crate::retrieve::estimate_tokens;

/// One candidate context chunk with cost and priority for budget selection.
#[derive(Debug, Clone)]
pub struct ContextCandidate {
    pub source_type: String,
    pub source_id: String,
    pub text: String,
    pub relevance: f64,
    /// Higher = evicted later.
    pub priority: u32,
    pub tokens: u32,
}

impl ContextCandidate {
    pub fn new(source_type: &str, source_id: &str, text: String, relevance: f64, priority: u32) -> Self {
        let tokens = estimate_tokens(&text);
        Self {
            source_type: source_type.into(),
            source_id: source_id.into(),
            text,
            relevance,
            priority,
            tokens,
        }
    }
}

/// Structured compiler input. All retrieved content is DATA.
#[derive(Debug, Clone, Default)]
pub struct PromptCompilerInput {
    pub system_kernel: String,
    pub role_prompt: String,
    pub task: String,
    pub skills: Vec<ContextCandidate>,
    pub workspace: Vec<ContextCandidate>,
    pub memory: Vec<ContextCandidate>,
    pub references: Vec<ContextCandidate>,
    pub checkpoint: String,
    pub recent_messages: Vec<String>,
    pub tool_results: Vec<ContextCandidate>,
}

/// Per-category token accounting for the debug view.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextBreakdown {
    pub system: u32,
    pub role: u32,
    pub skills: u32,
    pub workspace: u32,
    pub memory: u32,
    pub references: u32,
    pub checkpoint: u32,
    pub recent: u32,
    pub request: u32,
    pub tool_results: u32,
    pub output_reserve: u32,
    pub estimated_total: u32,
    pub model_limit: u32,
    pub remaining: i64,
}

impl ContextBreakdown {
    pub fn render(&self) -> String {
        format!(
            "Context Breakdown\n\nSystem          {:>7}\nRole            {:>7}\nSkills          {:>7}\nWorkspace       {:>7}\nMemory          {:>7}\nReferences      {:>7}\nCheckpoint      {:>7}\nRecent          {:>7}\nRequest         {:>7}\nTool results    {:>7}\nOutput reserve  {:>7}\n----------------------\nEstimated       {:>7}\n\nModel limit     {:>7}\n\nRemaining       {:>7}",
            self.system,
            self.role,
            self.skills,
            self.workspace,
            self.memory,
            self.references,
            self.checkpoint,
            self.recent,
            self.request,
            self.tool_results,
            self.output_reserve,
            self.estimated_total,
            self.model_limit,
            self.remaining,
        )
    }
}

#[derive(Debug, Clone)]
pub struct CompiledPrompt {
    pub system: String,
    pub user: String,
    pub breakdown: ContextBreakdown,
    /// True when reduction ran (some candidates dropped or compact forms used).
    pub reduced: bool,
}

pub struct PromptCompiler {
    pub model_limit: u32,
    pub output_reserve: u32,
}

impl PromptCompiler {
    pub fn new(model_limit: u32, output_reserve: u32) -> Self {
        Self { model_limit, output_reserve }
    }

    /// Select candidates within `budget`, ranked by (priority, relevance).
    /// Deterministic: ties broken by source_id.
    pub fn select_within_budget(candidates: &[ContextCandidate], budget: u32) -> Vec<ContextCandidate> {
        let mut sorted: Vec<&ContextCandidate> = candidates.iter().collect();
        sorted.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then(b.relevance.partial_cmp(&a.relevance).unwrap_or(std::cmp::Ordering::Equal))
                .then(a.source_id.cmp(&b.source_id))
        });
        let mut used = 0u32;
        let mut out = Vec::new();
        for c in sorted {
            if used + c.tokens <= budget {
                used += c.tokens;
                out.push((*c).clone());
            }
        }
        out
    }

    /// Compile the full prompt. Reduction order on overflow: references →
    /// tool results → memory → workspace → skills(full→compact handled by
    /// caller) → older recent messages. Checkpoint and recent tail are
    /// retained; system/role/request/output are reserved and never cut.
    pub fn compile(&self, input: PromptCompilerInput) -> CompiledPrompt {
        let system_t = estimate_tokens(&input.system_kernel);
        let role_t = estimate_tokens(&input.role_prompt);
        let request_t = estimate_tokens(&input.task);
        let checkpoint_t = estimate_tokens(&input.checkpoint);

        let reserved = system_t + role_t + request_t + checkpoint_t + self.output_reserve;
        let mut remaining = self.model_limit.saturating_sub(reserved);

        // Category budgets: split remaining dynamically. Skills bounded small,
        // recent messages reserved per OpenCode-style 25% rule, rest shared.
        let recent_budget = (remaining / 4).max(500);
        let skills_budget = 2_000.min(remaining / 8 + 500);
        let shared = remaining.saturating_sub(recent_budget + skills_budget);

        // Recent messages: newest-first fill within recent_budget.
        let mut recent_text = String::new();
        let mut recent_used = 0u32;
        for m in input.recent_messages.iter().rev() {
            let t = estimate_tokens(m);
            if recent_used + t <= recent_budget {
                recent_used += t;
                recent_text = format!("{m}\n{recent_text}");
            } else {
                break;
            }
        }

        let skills_sel = Self::select_within_budget(&input.skills, skills_budget);
        // Shared pool: workspace + memory + references + tool results compete.
        let mut shared_all: Vec<ContextCandidate> = Vec::new();
        shared_all.extend(input.workspace.iter().cloned());
        shared_all.extend(input.memory.iter().cloned());
        shared_all.extend(input.references.iter().cloned());
        shared_all.extend(input.tool_results.iter().cloned());
        let shared_sel = Self::select_within_budget(&shared_all, shared);

        let mut skills_t = 0u32;
        let mut workspace_t = 0u32;
        let mut memory_t = 0u32;
        let mut references_t = 0u32;
        let mut tools_t = 0u32;
        let mut skills_text = String::new();
        let mut workspace_text = String::new();
        let mut memory_text = String::new();
        let mut references_text = String::new();
        let mut tools_text = String::new();
        for c in &skills_sel {
            skills_t += c.tokens;
            skills_text.push_str(&format!("\n\n### Skill: {}\n{}", c.source_id, c.text));
        }
        for c in &shared_sel {
            match c.source_type.as_str() {
                "workspace" => {
                    workspace_t += c.tokens;
                    workspace_text.push_str(&format!("\n\n### Workspace: {}\n{}", c.source_id, c.text));
                }
                "memory" => {
                    memory_t += c.tokens;
                    memory_text.push_str(&format!("\n\n### Memory: {}\n{}", c.source_id, c.text));
                }
                "reference" => {
                    references_t += c.tokens;
                    references_text.push_str(&format!("\n\n### Reference: {}\n{}", c.source_id, c.text));
                }
                _ => {
                    tools_t += c.tokens;
                    tools_text.push_str(&format!("\n\n### Tool result: {}\n{}", c.source_id, c.text));
                }
            }
        }

        let reduced = skills_sel.len() < input.skills.len()
            || shared_sel.len() < shared_all.len()
            || recent_used
                < input
                    .recent_messages
                    .iter()
                    .map(|m| estimate_tokens(m))
                    .sum::<u32>();

        let system = format!(
            "{}\n\n---\n\n{}{}{}{}{}{}",
            input.system_kernel,
            input.role_prompt,
            skills_text,
            workspace_text,
            memory_text,
            references_text,
            if input.checkpoint.is_empty() {
                String::new()
            } else {
                format!("\n\n---\n\n## Checkpoint (session data)\n{}", input.checkpoint)
            },
        );
        let user = format!(
            "{}## Recent\n{}\n\n---\n\n## Request\n{}",
            if tools_text.is_empty() { String::new() } else { format!("## Tool results{tools_text}\n\n---\n\n") },
            recent_text.trim_end(),
            input.task
        );

        let estimated_total =
            system_t + role_t + skills_t + workspace_t + memory_t + references_t + checkpoint_t + recent_used + request_t + tools_t + self.output_reserve;
        let breakdown = ContextBreakdown {
            system: system_t,
            role: role_t,
            skills: skills_t,
            workspace: workspace_t,
            memory: memory_t,
            references: references_t,
            checkpoint: checkpoint_t,
            recent: recent_used,
            request: request_t,
            tool_results: tools_t,
            output_reserve: self.output_reserve,
            estimated_total,
            model_limit: self.model_limit,
            remaining: self.model_limit as i64 - estimated_total as i64,
        };
        CompiledPrompt { system, user, breakdown, reduced }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(t: &str, id: &str, rel: f64, pri: u32) -> ContextCandidate {
        ContextCandidate::new(t, id, format!("content of {id}"), rel, pri)
    }

    fn input() -> PromptCompilerInput {
        PromptCompilerInput {
            system_kernel: "SYS".into(),
            role_prompt: "ROLE".into(),
            task: "Fix the bug".into(),
            skills: vec![cand("skill", "testing", 0.9, 5)],
            workspace: vec![cand("workspace", "src/a.rs", 0.8, 5)],
            memory: vec![cand("memory", "pref-1", 0.5, 3)],
            references: vec![cand("reference", "opencode/compaction", 0.4, 2)],
            checkpoint: "plan done".into(),
            recent_messages: vec!["hello".into(), "world".into()],
            tool_results: vec![],
        }
    }

    #[test]
    fn ordering_system_role_skills_workspace_memory_reference_checkpoint_recent_request() {
        let c = PromptCompiler::new(32_000, 2_000);
        let out = c.compile(input());
        let sys = out.system.find("SYS").unwrap();
        let role = out.system.find("ROLE").unwrap();
        let skill = out.system.find("### Skill: testing").unwrap();
        let ws = out.system.find("### Workspace: src/a.rs").unwrap();
        let mem = out.system.find("### Memory: pref-1").unwrap();
        let rf = out.system.find("### Reference: opencode/compaction").unwrap();
        let ck = out.system.find("## Checkpoint").unwrap();
        assert!(sys < role && role < skill && skill < ws && ws < mem && mem < rf && rf < ck);
        assert!(out.user.contains("## Request\nFix the bug"));
        assert!(out.user.contains("## Recent"));
    }

    #[test]
    fn budget_drops_low_priority_first() {
        // Tiny budget: only reserved + a sliver remain.
        let c = PromptCompiler::new(3_000, 2_000);
        let mut inp = input();
        inp.skills.push(cand("skill", "low-priority-skill-xyz", 0.01, 1));
        let out = c.compile(inp);
        assert!(out.reduced);
        // High-priority testing skill should survive; low one may drop.
        assert!(out.system.contains("testing") || !out.system.contains("low-priority"));
    }

    #[test]
    fn breakdown_sums_and_renders() {
        let c = PromptCompiler::new(32_000, 2_000);
        let out = c.compile(input());
        let b = &out.breakdown;
        assert_eq!(
            b.estimated_total,
            b.system + b.role + b.skills + b.workspace + b.memory + b.references + b.checkpoint + b.recent + b.request + b.tool_results + b.output_reserve
        );
        assert!(b.remaining > 0);
        let rendered = b.render();
        assert!(rendered.contains("Context Breakdown"));
        assert!(rendered.contains("Remaining"));
    }

    #[test]
    fn select_is_deterministic() {
        let items = vec![
            cand("skill", "b", 0.5, 5),
            cand("skill", "a", 0.5, 5),
            cand("skill", "c", 0.9, 1),
        ];
        let a = PromptCompiler::select_within_budget(&items, 10_000);
        let b = PromptCompiler::select_within_budget(&items, 10_000);
        let ids_a: Vec<&str> = a.iter().map(|c| c.source_id.as_str()).collect();
        let ids_b: Vec<&str> = b.iter().map(|c| c.source_id.as_str()).collect();
        assert_eq!(ids_a, ids_b);
    }
}
