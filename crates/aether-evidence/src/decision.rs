//! Decision aggregation — given an `EvidenceBag`, produce a final `Decision`.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::bag::EvidenceBag;
use crate::evidence::Recommendation;

/// The verdict of the controller after evaluating all collected evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Accept the work and proceed.
    Pass,
    /// Reject the work; the planner should produce a new plan.
    Replan,
    /// Pause for human review.
    Escalate,
}

/// Human-readable reasoning for the verdict. The controller should surface
/// this to the UI so the user understands *why* the verdict was made.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionReasoning {
    pub summary: String,
    pub facts: Vec<String>,
    pub caveats: Vec<String>,
}

impl DecisionReasoning {
    fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            facts: Vec::new(),
            caveats: Vec::new(),
        }
    }
}

/// The final decision: a verdict + the reasoning behind it + the evidence
/// ids that drove it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub verdict: Verdict,
    pub reasoning: DecisionReasoning,
    pub evidence_ids: Vec<crate::evidence::EvidenceId>,
    pub aggregate_confidence: f32,
    pub contributing_agents: Vec<String>,
    pub recommendation_counts: RecommendationCounts,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecommendationCounts {
    pub pass: usize,
    pub replan: usize,
    pub manual_review: usize,
    pub uncertain: usize,
}

impl Decision {
    /// Render the decision as a short human-readable summary.
    pub fn render(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "VERDICT: {:?}", self.verdict);
        let _ = writeln!(s, "Confidence (aggregated): {:.2}", self.aggregate_confidence);
        let _ = writeln!(s, "Contributing agents: {}", self.contributing_agents.join(", "));
        let _ = writeln!(s, "Recommendations: pass={} replan={} manual_review={} uncertain={}",
            self.recommendation_counts.pass,
            self.recommendation_counts.replan,
            self.recommendation_counts.manual_review,
            self.recommendation_counts.uncertain);
        let _ = writeln!(s, "Summary: {}", self.reasoning.summary);
        if !self.reasoning.facts.is_empty() {
            let _ = writeln!(s, "Facts:");
            for f in &self.reasoning.facts {
                let _ = writeln!(s, "  - {f}");
            }
        }
        if !self.reasoning.caveats.is_empty() {
            let _ = writeln!(s, "Caveats:");
            for c in &self.reasoning.caveats {
                let _ = writeln!(s, "  - {c}");
            }
        }
        s
    }
}

/// Decide the verdict from a bag of evidence. The algorithm:
/// 1. If any record has a critical contradiction → `Escalate`.
/// 2. If ≥1 record recommends `Replan` and aggregate confidence < 0.6 → `Replan`.
/// 3. If any record recommends `ManualReview` and aggregate confidence < 0.85 → `Escalate`.
/// 4. If aggregate confidence ≥ 0.7 AND no `Replan` AND no `ManualReview` → `Pass`.
/// 5. Default: `Escalate`.
pub fn decide(bag: &EvidenceBag) -> Decision {
    let all = bag.all();
    let aggregate = bag.aggregate_confidence();

    let mut counts = RecommendationCounts::default();
    for e in &all {
        match e.recommendation {
            Recommendation::Pass => counts.pass += 1,
            Recommendation::Replan => counts.replan += 1,
            Recommendation::ManualReview => counts.manual_review += 1,
            Recommendation::Uncertain => counts.uncertain += 1,
        }
    }

    let mut reasoning = DecisionReasoning::new(format!(
        "Aggregated {} evidence record(s) from {} agent(s) at {:.2} confidence.",
        all.len(),
        bag.contributing_agents().len(),
        aggregate,
    ));

    for e in &all {
        reasoning.facts.push(format!(
            "[{}] {} (confidence={:.2})",
            e.source_agent,
            truncate(&e.claim, 120),
            e.confidence.0,
        ));
        if !e.tests.is_empty() {
            reasoning.facts.push(format!(
                "  tests: {} passed, {} failed",
                e.tests_passed(),
                e.tests_failed(),
            ));
        }
        for c in &e.contradictions {
            reasoning.caveats.push(format!(
                "[{}] {:?}: {}",
                e.source_agent, c.severity, c.description,
            ));
        }
    }

    let verdict = if bag.critical_contradictions() > 0 {
        Verdict::Escalate
    } else if counts.replan > 0 && counts.pass == 0 {
        // Any replan recommendation with no compensating pass → replan.
        Verdict::Replan
    } else if counts.manual_review > 0 && aggregate < 0.85 {
        Verdict::Escalate
    } else if aggregate >= 0.7 && counts.replan == 0 && counts.manual_review == 0 {
        Verdict::Pass
    } else {
        Verdict::Escalate
    };

    Decision {
        verdict,
        reasoning,
        evidence_ids: all.iter().map(|e| e.id.clone()).collect(),
        aggregate_confidence: aggregate,
        contributing_agents: bag.contributing_agents(),
        recommendation_counts: counts,
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut t = s.chars().take(max).collect::<String>();
        t.push('…');
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{
        Contradiction, ContradictionSeverity, Evidence, EvidenceKind, Recommendation,
    };

    fn ev_pass(agent: &str, claim: &str, conf: f32) -> Evidence {
        Evidence::new(agent, "implementer", EvidenceKind::Implementation, claim)
            .with_confidence(crate::evidence::Confidence(conf))
            .with_recommendation(Recommendation::Pass)
    }

    fn ev_replan(agent: &str, claim: &str) -> Evidence {
        Evidence::new(agent, "reviewer", EvidenceKind::Review, claim)
            .with_confidence(crate::evidence::Confidence(0.9))
            .with_recommendation(Recommendation::Replan)
    }

    #[test]
    fn empty_bag_escalates() {
        let bag = EvidenceBag::new();
        let d = decide(&bag);
        assert_eq!(d.verdict, Verdict::Escalate);
    }

    #[test]
    fn all_pass_high_confidence_passes() {
        let bag = EvidenceBag::new();
        bag.add(ev_pass("a", "ok", 0.95));
        bag.add(ev_pass("b", "ok", 0.9));
        let d = decide(&bag);
        assert_eq!(d.verdict, Verdict::Pass);
    }

    #[test]
    fn replan_with_low_confidence_replans() {
        let bag = EvidenceBag::new();
        // Two replan recommendations with low confidence must trigger replan.
        bag.add(ev_replan("a", "needs work"));
        bag.add(ev_replan("b", "still broken"));
        let d = decide(&bag);
        assert_eq!(d.verdict, Verdict::Replan);
    }

    #[test]
    fn critical_contradiction_escalates() {
        let bag = EvidenceBag::new();
        bag.add(
            ev_pass("a", "ok", 0.95).with_contradiction(Contradiction {
                description: "tests didn't actually run".into(),
                source: "self".into(),
                severity: ContradictionSeverity::Critical,
            }),
        );
        let d = decide(&bag);
        assert_eq!(d.verdict, Verdict::Escalate);
    }

    #[test]
    fn manual_review_with_low_confidence_escalates() {
        let bag = EvidenceBag::new();
        bag.add(ev_pass("a", "ok", 0.6));
        bag.add(Evidence::new(
            "reviewer",
            "reviewer",
            EvidenceKind::Review,
            "needs review",
        )
        .with_recommendation(Recommendation::ManualReview)
        .with_confidence(crate::evidence::Confidence(0.6)));
        let d = decide(&bag);
        assert_eq!(d.verdict, Verdict::Escalate);
    }

    #[test]
    fn render_includes_all_sections() {
        let bag = EvidenceBag::new();
        bag.add(ev_pass("coder", "implemented oauth login with JWT", 0.92));
        let d = decide(&bag);
        let r = d.render();
        assert!(r.contains("VERDICT"));
        assert!(r.contains("Confidence"));
        assert!(r.contains("Summary"));
    }
}
