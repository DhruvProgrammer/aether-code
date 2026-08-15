//! Loop engineering: an explicit `EngineeringModel` plus a `LoopEngine` that maintains it
//! across the agent's closed loop (spec: "the AI must understand what it is doing").
//!
//! The model is the single source of truth the Controller/Executor reason against: goal,
//! facts, unknowns, hypotheses (with confidence + validation), evidence, observations,
//! decisions, failures, risks, the current strategy and the next-best-action. The engine
//! detects stagnation, enforces budgets, computes confidence, and decides whether to
//! continue, escalate, or stop — a mechanical circuit breaker that prevents blind retries.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn normalize(s: &str) -> String {
    let lower = s.to_lowercase();
    // Collapse volatile parts (digit runs, long hashes) but keep structural words and
    // paths so that *structurally identical* failures across attempts share a signature
    // while genuinely different errors remain distinct.
    lower
        .split(|c: char| c.is_whitespace() || c == ':')
        .filter(|t| !t.is_empty())
        .map(|tok| {
            let no_digits: String = tok.chars().map(|c| if c.is_ascii_digit() { '0' } else { c }).collect();
            if no_digits.chars().all(|c| c == '0') {
                "{n}".to_string()
            } else if no_digits.chars().all(|c| c.is_alphanumeric()) && no_digits.len() > 12 {
                "{hash}".to_string()
            } else {
                no_digits
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum LoopState {
    #[default]
    Scheduled,
    Understanding,
    Designing,
    Implementing,
    Verifying,
    Decision,
    Closed,
    Escalated,
    Stalled,
}

impl LoopState {
    pub fn label(&self) -> &'static str {
        match self {
            LoopState::Scheduled => "SCHEDULED",
            LoopState::Understanding => "UNDERSTANDING",
            LoopState::Designing => "DESIGNING",
            LoopState::Implementing => "IMPLEMENTING",
            LoopState::Verifying => "VERIFYING",
            LoopState::Decision => "DECISION",
            LoopState::Closed => "CLOSED",
            LoopState::Escalated => "ESCALATED",
            LoopState::Stalled => "STALLED",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum HypothesisStatus {
    #[default]
    Active,
    Verified,
    Refuted,
    Parked,
}

impl HypothesisStatus {
    pub fn label(&self) -> &'static str {
        match self {
            HypothesisStatus::Active => "active",
            HypothesisStatus::Verified => "verified",
            HypothesisStatus::Refuted => "refuted",
            HypothesisStatus::Parked => "parked",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Hypothesis {
    pub id: String,
    pub statement: String,
    pub confidence: f32,
    pub status: HypothesisStatus,
    pub validation_action: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Evidence {
    pub id: String,
    pub description: String,
    pub source: String,
    pub supports: Option<String>,
    pub contradicts: Option<String>,
    pub confidence: f32,
    pub ts: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Observation {
    pub ts: String,
    pub source: String,
    pub summary: String,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub matched: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Decision {
    pub ts: String,
    pub choice: String,
    pub rationale: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Action {
    pub ts: String,
    pub description: String,
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Failure {
    pub ts: String,
    pub signature: String,
    pub detail: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Risk {
    pub description: String,
    pub severity: String,
    pub likelihood: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EngineeringModel {
    pub goal: String,
    pub success_criteria: Vec<String>,
    pub met_criteria: Vec<String>,
    pub facts: Vec<String>,
    pub assumptions: Vec<String>,
    pub unknowns: Vec<String>,
    pub hypotheses: Vec<Hypothesis>,
    pub evidence: Vec<Evidence>,
    pub observations: Vec<Observation>,
    pub decisions: Vec<Decision>,
    pub actions: Vec<Action>,
    pub failures: Vec<Failure>,
    pub risks: Vec<Risk>,
    pub current_strategy: Option<String>,
    pub next_best_action: Option<String>,
    pub confidence: f32,
    pub risk_level: String,
    pub loop_state: LoopState,
    pub iteration: u32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopAction {
    Continue,
    Escalate,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetStatus {
    Ok,
    Warn,
    HardStop,
}

pub struct LoopEngine {
    pub model: EngineeringModel,
    pub stagnation_threshold: usize,
    pub confidence_escalation: f32,
    pub max_failures: usize,
    pub high_risk_severity: HashSet<String>,
    consecutive_failures: usize,
    last_signature: Option<String>,
    consecutive_same_action: usize,
    last_action: Option<String>,
}

impl LoopEngine {
    pub fn new(goal: &str) -> Self {
        let ts = now();
        Self {
            model: EngineeringModel {
                goal: goal.to_string(),
                loop_state: LoopState::Scheduled,
                created_at: ts.clone(),
                updated_at: ts,
                risk_level: "low".into(),
                ..Default::default()
            },
            stagnation_threshold: 3,
            confidence_escalation: 0.4,
            max_failures: 5,
            high_risk_severity: {
                let mut s = HashSet::new();
                s.insert("high".to_string());
                s
            },
            consecutive_failures: 0,
            last_signature: None,
            consecutive_same_action: 0,
            last_action: None,
        }
    }

    pub fn with_thresholds(mut self, stagnation: usize, confidence_escalation: f32, max_failures: usize) -> Self {
        self.stagnation_threshold = stagnation.max(1);
        self.confidence_escalation = confidence_escalation.clamp(0.0, 1.0);
        self.max_failures = max_failures.max(1);
        self
    }

    fn touch(&mut self) {
        self.model.updated_at = now();
    }

    pub fn set_success_criteria(&mut self, criteria: Vec<String>) {
        self.model.success_criteria = criteria;
        self.touch();
    }

    pub fn mark_criteria_met(&mut self, criterion: &str) {
        if !self.model.met_criteria.iter().any(|c| c == criterion) {
            self.model.met_criteria.push(criterion.to_string());
        }
        self.touch();
    }

    pub fn add_fact(&mut self, fact: &str) {
        if !self.model.facts.iter().any(|f| f == fact) {
            self.model.facts.push(fact.to_string());
        }
        self.touch();
    }

    pub fn add_assumption(&mut self, assumption: &str) {
        if !self.model.assumptions.iter().any(|a| a == assumption) {
            self.model.assumptions.push(assumption.to_string());
        }
        self.touch();
    }

    pub fn add_unknown(&mut self, unknown: &str) {
        if !self.model.unknowns.iter().any(|u| u == unknown) {
            self.model.unknowns.push(unknown.to_string());
        }
        self.touch();
    }

    pub fn add_risk(&mut self, description: &str, severity: &str, likelihood: f32) {
        self.model.risks.push(Risk {
            description: description.to_string(),
            severity: severity.to_string(),
            likelihood: likelihood.clamp(0.0, 1.0),
        });
        self.touch();
    }

    pub fn add_hypothesis(&mut self, statement: &str, confidence: f32, validation_action: Option<&str>) -> String {
        let id = format!("h{}", self.model.hypotheses.len() + 1);
        let ts = now();
        self.model.hypotheses.push(Hypothesis {
            id: id.clone(),
            statement: statement.to_string(),
            confidence: confidence.clamp(0.0, 1.0),
            status: HypothesisStatus::Active,
            validation_action: validation_action.map(str::to_string),
            created_at: ts.clone(),
            updated_at: ts,
        });
        self.touch();
        id
    }

    pub fn set_hypothesis_status(&mut self, id: &str, status: HypothesisStatus, confidence: Option<f32>) {
        if let Some(h) = self.model.hypotheses.iter_mut().find(|h| h.id == id) {
            h.status = status;
            if let Some(c) = confidence {
                h.confidence = c.clamp(0.0, 1.0);
            }
            h.updated_at = now();
        }
        self.touch();
    }

    pub fn add_evidence(
        &mut self,
        description: &str,
        source: &str,
        confidence: f32,
        supports: Option<&str>,
        contradicts: Option<&str>,
    ) {
        let id = format!("e{}", self.model.evidence.len() + 1);
        self.model.evidence.push(Evidence {
            id,
            description: description.to_string(),
            source: source.to_string(),
            supports: supports.map(str::to_string),
            contradicts: contradicts.map(str::to_string),
            confidence: confidence.clamp(0.0, 1.0),
            ts: now(),
        });
        if let Some(h) = contradicts {
            self.set_hypothesis_status(h, HypothesisStatus::Refuted, Some(0.1));
        }
        if let Some(h) = supports {
            if let Some(hyp) = self.model.hypotheses.iter_mut().find(|x| x.id == h) {
                hyp.status = HypothesisStatus::Verified;
                hyp.confidence = hyp.confidence.max(confidence.clamp(0.0, 1.0));
                hyp.updated_at = now();
            }
        }
        self.touch();
    }

    pub fn add_decision(&mut self, choice: &str, rationale: &str, confidence: f32) {
        self.model.decisions.push(Decision {
            ts: now(),
            choice: choice.to_string(),
            rationale: rationale.to_string(),
            confidence: confidence.clamp(0.0, 1.0),
        });
        self.touch();
    }

    pub fn set_strategy(&mut self, strategy: &str) {
        self.model.current_strategy = Some(strategy.to_string());
        self.model.loop_state = LoopState::Designing;
        self.touch();
    }

    pub fn set_next_best_action(&mut self, action: &str) {
        self.model.next_best_action = Some(action.to_string());
        self.touch();
    }

    pub fn record_action(&mut self, description: &str) {
        if self.last_action.as_deref() == Some(description) {
            self.consecutive_same_action += 1;
        } else {
            self.consecutive_same_action = 1;
            self.last_action = Some(description.to_string());
        }
        self.model.actions.push(Action {
            ts: now(),
            description: description.to_string(),
            outcome: None,
        });
        if self.model.actions.len() > 50 {
            let excess = self.model.actions.len() - 50;
            self.model.actions.drain(0..excess);
        }
        self.model.loop_state = LoopState::Implementing;
        self.touch();
    }

    pub fn observe(&mut self, source: &str, summary: &str, expected: Option<&str>, actual: Option<&str>) {
        let matched = match (expected, actual) {
            (Some(e), Some(a)) => Some(normalize(e) == normalize(a)),
            _ => None,
        };
        if matched == Some(false) {
            debug_assert!(expected.is_some() && actual.is_some(), "matched == Some(false) requires both expected and actual to be Some");
            self.add_unknown(&format!("Expected {} but observed {}", expected.unwrap(), actual.unwrap()));
        }
        self.model.observations.push(Observation {
            ts: now(),
            source: source.to_string(),
            summary: summary.to_string(),
            expected: expected.map(str::to_string),
            actual: actual.map(str::to_string),
            matched,
        });
        if self.model.observations.len() > 50 {
            let excess = self.model.observations.len() - 50;
            self.model.observations.drain(0..excess);
        }
        self.model.loop_state = LoopState::Verifying;
        self.touch();
    }

    pub fn note_failure(&mut self, detail: &str) {
        let sig = normalize(detail);
        if self.last_signature.as_deref() == Some(&sig) {
            self.consecutive_failures += 1;
            if let Some(f) = self.model.failures.iter_mut().find(|f| f.signature == sig) {
                f.count += 1;
                f.detail = detail.to_string();
                f.ts = now();
            }
        } else {
            self.consecutive_failures = 1;
            self.last_signature = Some(sig.clone());
            self.model.failures.push(Failure {
                ts: now(),
                signature: sig,
                detail: detail.to_string(),
                count: 1,
            });
        }
        if self.model.failures.len() > 20 {
            let excess = self.model.failures.len() - 20;
            self.model.failures.drain(0..excess);
        }
        self.model.loop_state = LoopState::Implementing;
        self.touch();
    }

    pub fn recompute(&mut self) {
        self.model.confidence = self.compute_confidence();
        self.model.risk_level = self.compute_risk_level();
        self.touch();
    }

    fn compute_confidence(&self) -> f32 {
        if self.model.hypotheses.is_empty() {
            // No hypotheses yet: neutral uncertainty.
            return 0.4;
        }
        let total = self.model.hypotheses.len() as f32;
        let verified = self.model.hypotheses.iter().filter(|h| h.status == HypothesisStatus::Verified).count() as f32;
        let refuted = self.model.hypotheses.iter().filter(|h| h.status == HypothesisStatus::Refuted).count() as f32;
        let avg_conf = self.model.hypotheses.iter().map(|h| h.confidence).sum::<f32>() / total;
        let mut c = 0.4 * avg_conf + 0.4 * (verified / total) - 0.3 * (refuted / total);
        let fail_count: usize = self.model.failures.iter().map(|f| f.count).sum();
        c -= (fail_count as f32) * 0.05;
        c.clamp(0.0, 1.0)
    }

    fn compute_risk_level(&self) -> String {
        let high = self.model.risks.iter().any(|r| self.high_risk_severity.contains(&r.severity))
            || self.model.failures.iter().map(|f| f.count).sum::<usize>() >= self.max_failures;
        let medium = self.model.risks.iter().any(|r| r.severity == "medium")
            || !self.model.failures.is_empty()
            || !self.model.unknowns.is_empty();
        if high {
            "high".into()
        } else if medium {
            "medium".into()
        } else {
            "low".into()
        }
    }

    pub fn detect_stagnation(&self) -> bool {
        self.consecutive_failures >= self.stagnation_threshold
            || self.consecutive_same_action >= self.stagnation_threshold * 2
    }

    pub fn should_escalate(&self) -> bool {
        let low_conf = self.model.confidence < self.confidence_escalation;
        let high_risk = self.model.risk_level == "high"
            || self.model.risks.iter().any(|r| self.high_risk_severity.contains(&r.severity));
        let refuted = self.model.hypotheses.iter().any(|h| h.status == HypothesisStatus::Refuted);
        low_conf && (high_risk || refuted)
    }

    pub fn success_met(&self) -> bool {
        !self.model.success_criteria.is_empty()
            && self.model.success_criteria.iter().all(|c| self.model.met_criteria.iter().any(|m| m == c))
    }

    pub fn budget_status(&self, iteration: u32, max: u32) -> BudgetStatus {
        if max == 0 {
            return BudgetStatus::HardStop;
        }
        let ratio = iteration as f32 / max as f32;
        if iteration >= max {
            BudgetStatus::HardStop
        } else if ratio >= 0.9 {
            BudgetStatus::Warn
        } else if ratio >= 0.8 {
            BudgetStatus::Warn
        } else {
            BudgetStatus::Ok
        }
    }

    /// The mechanical circuit breaker: combine budget, stagnation, escalation and
    /// success into a single decision for the outer loop.
    pub fn decide(&mut self, iteration: u32, max: u32) -> LoopAction {
        self.recompute();
        if matches!(self.budget_status(iteration, max), BudgetStatus::HardStop) {
            self.model.loop_state = LoopState::Stalled;
            return LoopAction::Stop;
        }
        if self.detect_stagnation() {
            // Stuck on a repeating pattern: never blind-retry. Surface the briefing.
            self.model.loop_state = LoopState::Stalled;
            return LoopAction::Escalate;
        }
        if self.should_escalate() {
            self.model.loop_state = LoopState::Escalated;
            return LoopAction::Escalate;
        }
        if self.success_met() {
            self.model.loop_state = LoopState::Closed;
            return LoopAction::Stop;
        }
        self.model.loop_state = LoopState::Decision;
        LoopAction::Continue
    }

    pub fn state_summary(&self) -> String {
        let m = &self.model;
        let mut s = String::new();
        s.push_str(&format!("Goal: {}\n", m.goal));
        s.push_str(&format!(
            "State: {} | Confidence: {:.0}% | Risk: {}\n",
            m.loop_state.label(),
            m.confidence * 100.0,
            m.risk_level
        ));
        if !m.facts.is_empty() {
            s.push_str("Facts:\n");
            for f in &m.facts {
                s.push_str(&format!("  - {f}\n"));
            }
        }
        if !m.unknowns.is_empty() {
            s.push_str("Unknowns:\n");
            for u in &m.unknowns {
                s.push_str(&format!("  - {u}\n"));
            }
        }
        if !m.hypotheses.is_empty() {
            s.push_str("Hypotheses:\n");
            for h in &m.hypotheses {
                s.push_str(&format!(
                    "  - [{}] {} (conf {:.0}%)\n",
                    h.status.label(),
                    h.statement,
                    h.confidence * 100.0
                ));
            }
        }
        if !m.failures.is_empty() {
            s.push_str("Failures:\n");
            for f in &m.failures {
                s.push_str(&format!("  - (x{}) {}\n", f.count, f.detail));
            }
        }
        if let Some(n) = &m.next_best_action {
            s.push_str(&format!("Next best action: {n}\n"));
        }
        s
    }

    /// Compact, human-readable engineering run panel for the CLI/UI.
    pub fn render_panel(&self) -> String {
        let m = &self.model;
        let mut s = String::new();
        s.push_str("╭─ AETHER ENGINEERING RUN ───────────────────────────────\n");
        s.push_str(&format!(
            "│ State: {}  Conf: {:.0}%  Risk: {}\n",
            m.loop_state.label(),
            m.confidence * 100.0,
            m.risk_level.to_uppercase()
        ));
        s.push_str(&format!("│ Goal: {}\n", m.goal));
        if !m.current_strategy.as_deref().unwrap_or("").is_empty() {
            let strat: String = m.current_strategy.as_deref().unwrap_or("").chars().take(80).collect();
            s.push_str(&format!("│ Strategy: {}\n", strat));
        }
        if !m.hypotheses.is_empty() {
            s.push_str(&format!("│ Hypotheses: {} ({} verified, {} refuted)\n",
                m.hypotheses.len(),
                m.hypotheses.iter().filter(|h| h.status == HypothesisStatus::Verified).count(),
                m.hypotheses.iter().filter(|h| h.status == HypothesisStatus::Refuted).count()));
        }
        if !m.failures.is_empty() {
            let total: usize = m.failures.iter().map(|f| f.count).sum();
            s.push_str(&format!("│ Failures: {} (last: {})\n", total,
                m.failures.last().map(|f| f.detail.as_str()).unwrap_or("")));
        }
        if let Some(n) = &m.next_best_action {
            s.push_str(&format!("│ Next: {}\n", n));
        }
        s.push_str("╰───────────────────────────────────────────────────────\n");
        s
    }

    /// Produce a structured escalation briefing for the human when the loop gives up.
    pub fn escalation_briefing(&self) -> String {
        let mut s = String::new();
        s.push_str("\n[ESCALATION] The engineering loop stopped without meeting success criteria.\n");
        s.push_str(&self.state_summary());
        s.push_str("\nRecommended human input:\n");
        if !self.model.unknowns.is_empty() {
            s.push_str("- Resolve the open unknowns above.\n");
        }
        if !self.model.failures.is_empty() {
            s.push_str("- Inspect the recurring failures; the current approach is not converging.\n");
        }
        if self.model.risk_level == "high" {
            s.push_str("- A high-risk condition was detected; confirm before retrying.\n");
        }
        s.push_str("- Provide additional context, constraints, or approve a riskier approach.\n");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_model_starts_scheduled() {
        let e = LoopEngine::new("fix the bug");
        assert_eq!(e.model.loop_state, LoopState::Scheduled);
        assert_eq!(e.model.goal, "fix the bug");
        assert_eq!(e.model.confidence, 0.0);
    }

    #[test]
    fn hypotheses_drive_confidence() {
        let mut e = LoopEngine::new("t");
        let h1 = e.add_hypothesis("root cause is X", 0.8, Some("run test"));
        e.add_evidence("test fails at X", "test", 0.9, Some(&h1), None);
        e.recompute();
        assert!(e.model.confidence > 0.5);
        assert_eq!(e.model.hypotheses[0].status, HypothesisStatus::Verified);
    }

    #[test]
    fn refutation_lowers_and_escalates() {
        let mut e = LoopEngine::new("t");
        let h1 = e.add_hypothesis("approach A works", 0.6, None);
        e.add_evidence("approach A failed", "review", 0.9, None, Some(&h1));
        e.add_risk("data loss possible", "high", 0.7);
        e.recompute();
        assert_eq!(e.model.hypotheses[0].status, HypothesisStatus::Refuted);
        assert!(e.should_escalate());
    }

    #[test]
    fn stagnation_detected_on_repeated_failures() {
        let mut e = LoopEngine::new("t");
        e.note_failure("compile error at src/main.rs:42");
        e.note_failure("compile error at src/main.rs:42");
        e.note_failure("compile error at src/main.rs:42");
        assert!(e.detect_stagnation());
        assert_eq!(e.model.failures.len(), 1);
        assert_eq!(e.model.failures[0].count, 3);
    }

    #[test]
    fn different_failures_do_not_stagnate() {
        let mut e = LoopEngine::new("t");
        e.note_failure("error A at line 1");
        e.note_failure("error B at line 2");
        e.note_failure("error C at line 3");
        assert!(!e.detect_stagnation());
    }

    #[test]
    fn circuit_breaker_stops_on_hard_budget() {
        let mut e = LoopEngine::new("t");
        assert_eq!(e.decide(3, 3), LoopAction::Stop);
        assert_eq!(e.model.loop_state, LoopState::Stalled);
    }

    #[test]
    fn success_criteria_close_the_loop() {
        let mut e = LoopEngine::new("t");
        e.set_success_criteria(vec!["reviewer passes".into(), "tester passes".into()]);
        e.mark_criteria_met("reviewer passes");
        e.mark_criteria_met("tester passes");
        assert!(e.success_met());
        assert_eq!(e.decide(1, 3), LoopAction::Stop);
        assert_eq!(e.model.loop_state, LoopState::Closed);
    }

    #[test]
    fn escalation_on_stagnation_with_low_confidence() {
        let mut e = LoopEngine::with_thresholds(LoopEngine::new("t"), 3, 0.4, 5);
        e.note_failure("same error");
        e.note_failure("same error");
        e.note_failure("same error");
        assert_eq!(e.decide(1, 5), LoopAction::Escalate);
    }
}
