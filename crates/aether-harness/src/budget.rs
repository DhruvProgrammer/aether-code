//! Bounded autonomous execution (spec §17).
//!
//! Four independent budgets: turns, tokens, wall-clock, tool calls. Reaching
//! a limit is **never** success — the reason type keeps that distinction
//! explicit so callers cannot accidentally report a budget stop as a
//! completed goal (spec §17, and the reference harness's two-authority rule:
//! gates decide success, budgets decide exhaustion).

use serde::{Deserialize, Serialize};

/// Sentinel meaning "no cap". Kept well below `i64::MAX`/JSON limits.
pub const UNLIMITED: u64 = 9_007_199_254_740_991;

/// Why autonomous execution stopped. `None` never means success.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum LimitReason {
    Turns { used: u32, limit: u32 },
    Tokens { used: u64, limit: u64 },
    Time { secs: u64, limit: u64 },
    ToolCalls { used: u32, limit: u32 },
}

impl LimitReason {
    /// Human-readable, mirrors `describe_autonomous_limit` in the reference.
    pub fn describe(&self) -> String {
        match self {
            Self::Turns { used, limit } => format!("maxTurns reached ({used}/{limit})"),
            Self::Tokens { used, limit } => format!("maxTokens reached ({used}/{limit})"),
            Self::Time { secs, limit } => format!("timeout reached ({secs}s/{limit}s)"),
            Self::ToolCalls { used, limit } => {
                format!("maxToolCalls reached ({used}/{limit})")
            }
        }
    }

    /// Always false: exhaustion is not completion.
    pub fn is_success(&self) -> bool {
        false
    }
}

/// Budget limits. `0` (or `UNLIMITED`) disables that dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    pub max_turns: u32,
    pub max_tokens: u64,
    pub max_secs: u64,
    pub max_tool_calls: u32,
    /// Sub-second wall-clock limit. `0` means "use `max_secs`". Exists so
    /// fast tests (and sub-minute tasks) can still bound execution time.
    pub max_millis: u64,
}

impl Default for Budget {
    fn default() -> Self {
        // Conservative defaults; the CLI can raise them.
        Self {
            max_turns: 12,
            max_tokens: 80_000,
            max_secs: 30 * 60,
            max_tool_calls: 0,
            max_millis: 0,
        }
    }
}

impl Budget {
    pub fn unlimited() -> Self {
        Self {
            max_turns: 0,
            max_tokens: UNLIMITED,
            max_secs: 0,
            max_tool_calls: 0,
            max_millis: 0,
        }
    }

    pub fn turns(n: u32) -> Self {
        Self {
            max_turns: n,
            ..Default::default()
        }
    }

    /// Effective wall-clock limit in milliseconds, or `None` when unbounded.
    pub fn wall_clock_ms(&self) -> Option<u64> {
        if self.max_millis > 0 {
            Some(self.max_millis)
        } else if self.max_secs > 0 {
            Some(self.max_secs.saturating_mul(1000))
        } else {
            None
        }
    }
}

/// Accumulated spend. Counted by the runtime, not inferred.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetUsage {
    pub turns: u32,
    pub tokens: u64,
    pub tool_calls: u32,
    pub started_at: i64,
}

impl BudgetUsage {
    pub fn new(now: i64) -> Self {
        Self {
            started_at: now,
            ..Default::default()
        }
    }

    pub fn elapsed_secs(&self, now: i64) -> u64 {
        (now - self.started_at).max(0) as u64
    }

    pub fn add_turn(&mut self) {
        self.turns = self.turns.saturating_add(1);
    }

    pub fn add_tokens(&mut self, tokens: u64) {
        self.tokens = self.tokens.saturating_add(tokens);
    }

    pub fn add_tool_calls(&mut self, n: u32) {
        self.tool_calls = self.tool_calls.saturating_add(n);
    }
}

impl Budget {
    /// First exhausted dimension, in fixed order (turns → tokens → time →
    /// tool calls). Deterministic so tests and callers agree.
    pub fn exhausted(&self, usage: &BudgetUsage, now: i64) -> Option<LimitReason> {
        if self.max_turns > 0 && usage.turns >= self.max_turns {
            return Some(LimitReason::Turns {
                used: usage.turns,
                limit: self.max_turns,
            });
        }
        if self.max_tokens > 0 && usage.tokens >= self.max_tokens {
            return Some(LimitReason::Tokens {
                used: usage.tokens,
                limit: self.max_tokens,
            });
        }
        if self.max_secs > 0 {
            let secs = usage.elapsed_secs(now);
            if secs >= self.max_secs {
                return Some(LimitReason::Time {
                    secs,
                    limit: self.max_secs,
                });
            }
        }
        if self.max_tool_calls > 0 && usage.tool_calls >= self.max_tool_calls {
            return Some(LimitReason::ToolCalls {
                used: usage.tool_calls,
                limit: self.max_tool_calls,
            });
        }
        None
    }

    /// Fraction of each dimension consumed, for progress panels.
    pub fn utilisation(&self, usage: &BudgetUsage, now: i64) -> f32 {
        let mut total = 0.0f32;
        let mut dims = 0;
        if self.max_turns > 0 {
            total += usage.turns as f32 / self.max_turns as f32;
            dims += 1;
        }
        if self.max_tokens > 0 && self.max_tokens < UNLIMITED {
            total += usage.tokens as f32 / self.max_tokens as f32;
            dims += 1;
        }
        if self.max_secs > 0 {
            total += usage.elapsed_secs(now) as f32 / self.max_secs as f32;
            dims += 1;
        }
        if self.max_tool_calls > 0 {
            total += usage.tool_calls as f32 / self.max_tool_calls as f32;
            dims += 1;
        }
        if dims == 0 {
            0.0
        } else {
            total / dims as f32
        }
    }
}

/// How a bounded autonomous run ended (spec §17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum AutonomousOutcome {
    /// Goal satisfied **and** every required gate actually passed.
    GoalComplete,
    /// All gates passed.
    GatesPassed,
    /// A budget ran out. Explicitly not success.
    BudgetExhausted(LimitReason),
    /// A gate kept failing after its retry window.
    GateRetryExhausted { command: String, attempts: u32 },
    /// Blocked (needs user input or permission).
    Blocked { reason: String },
    /// Critical failure; state preserved for recovery.
    Failed { reason: String },
}

impl AutonomousOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::GoalComplete | Self::GatesPassed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_first_dimension_wins() {
        let b = Budget::default();
        let mut u = BudgetUsage::new(0);
        assert!(b.exhausted(&u, 0).is_none());
        for _ in 0..11 {
            u.add_turn();
        }
        assert!(b.exhausted(&u, 0).is_none());
        u.add_turn();
        assert_eq!(
            b.exhausted(&u, 0),
            Some(LimitReason::Turns { used: 12, limit: 12 })
        );
    }

    #[test]
    fn token_and_time_and_toolcall_limits() {
        let b = Budget {
            max_turns: 0,
            max_tokens: 100,
            max_secs: 60,
            max_tool_calls: 2,
            max_millis: 0,
        };
        let mut u = BudgetUsage::new(0);
        u.add_tokens(100);
        assert_eq!(
            b.exhausted(&u, 0),
            Some(LimitReason::Tokens { used: 100, limit: 100 })
        );
        u.tokens = 0;
        assert_eq!(b.exhausted(&u, 61), Some(LimitReason::Time { secs: 61, limit: 60 }));
        u.started_at = 0;
        u.add_tool_calls(2);
        assert_eq!(
            b.exhausted(&u, 0),
            Some(LimitReason::ToolCalls { used: 2, limit: 2 })
        );
    }

    #[test]
    fn unlimited_budget_never_stops() {
        let b = Budget::unlimited();
        let mut u = BudgetUsage::new(0);
        for _ in 0..1000 {
            u.add_turn();
            u.add_tokens(1_000_000);
            u.add_tool_calls(50);
        }
        assert!(b.exhausted(&u, 10_000).is_none());
    }

    #[test]
    fn exhaustion_is_never_success() {
        let r = LimitReason::Turns { used: 12, limit: 12 };
        assert!(!r.is_success());
        assert!(r.describe().contains("12/12"));
        assert!(!AutonomousOutcome::BudgetExhausted(r).is_success());
        assert!(AutonomousOutcome::GatesPassed.is_success());
        assert!(!AutonomousOutcome::Blocked { reason: "x".into() }.is_success());
    }
}
