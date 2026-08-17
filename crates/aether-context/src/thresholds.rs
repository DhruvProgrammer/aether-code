//! Compaction thresholds.
//!
//! Three thresholds expressed as fractions of `context_window`:
//!   * warn       — surface a notice; do NOT compact.
//!   * compact    — proactively compact on the next opportunity.
//!   * emergency  — compact immediately, evict aggressively.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ContextThresholds {
    /// Fraction (0..=1) at which we surface a warning to the UI.
    pub warn: f32,
    /// Fraction at which we proactively compact.
    pub compact: f32,
    /// Fraction at which we compact immediately + evict aggressively.
    pub emergency: f32,
}

impl Default for ContextThresholds {
    fn default() -> Self {
        Self {
            warn: 0.70,
            compact: 0.82,
            emergency: 0.94,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThresholdAction {
    None,
    Warn,
    Compact,
    Emergency,
}

impl ContextThresholds {
    /// Classify a token usage fraction.
    pub fn classify(&self, pct: f32) -> ThresholdAction {
        if pct >= self.emergency {
            ThresholdAction::Emergency
        } else if pct >= self.compact {
            ThresholdAction::Compact
        } else if pct >= self.warn {
            ThresholdAction::Warn
        } else {
            ThresholdAction::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_routes_thresholds() {
        let t = ContextThresholds::default();
        assert_eq!(t.classify(0.50), ThresholdAction::None);
        assert_eq!(t.classify(0.75), ThresholdAction::Warn);
        assert_eq!(t.classify(0.85), ThresholdAction::Compact);
        assert_eq!(t.classify(0.96), ThresholdAction::Emergency);
    }
}
