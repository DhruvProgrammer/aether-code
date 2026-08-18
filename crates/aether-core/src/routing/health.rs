//! Live health scoring for a model / provider.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// A snapshot of a model's recent health.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthScore {
    /// Most recent measured latency in milliseconds.
    pub latency_ms: u32,
    /// Rolling success rate over the last N calls (0.0..=1.0).
    pub success_rate: f32,
    /// Last health check outcome (None = unknown).
    pub last_check_ok: Option<bool>,
    /// When the last health check ran.
    pub last_check_at: Option<std::time::SystemTime>,
}

impl HealthScore {
    pub fn unknown() -> Self {
        Self {
            latency_ms: 0,
            success_rate: 1.0,
            last_check_ok: None,
            last_check_at: None,
        }
    }

    /// Score multiplier used by the router. < 1.0 down-weights unhealthy models.
    pub fn multiplier(&self) -> f32 {
        let latency_factor = if self.latency_ms == 0 {
            1.0
        } else if self.latency_ms < 500 {
            1.0
        } else if self.latency_ms < 2_000 {
            0.9
        } else if self.latency_ms < 10_000 {
            0.7
        } else {
            0.4
        };
        let success_factor = self.success_rate.max(0.0);
        latency_factor * success_factor
    }
}

/// Live health tracker per model. The router writes outcomes after each call;
/// the score decays over time.
#[derive(Debug)]
pub struct ModelHealth {
    model_id: String,
    recent_latency_ms: AtomicU32,
    successes: AtomicU32,
    failures: AtomicU32,
    last_check: parking_lot::Mutex<Option<(Instant, bool)>>,
}

impl ModelHealth {
    pub fn new(model_id: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            model_id: model_id.into(),
            recent_latency_ms: AtomicU32::new(0),
            successes: AtomicU32::new(0),
            failures: AtomicU32::new(0),
            last_check: parking_lot::Mutex::new(None),
        })
    }

    pub fn record_outcome(&self, success: bool, latency_ms: u32) {
        self.recent_latency_ms.store(latency_ms, Ordering::Relaxed);
        if success {
            self.successes.fetch_add(1, Ordering::Relaxed);
        } else {
            self.failures.fetch_add(1, Ordering::Relaxed);
        }
        *self.last_check.lock() = Some((Instant::now(), success));
    }

    pub fn snapshot(&self) -> HealthScore {
        let s = self.successes.load(Ordering::Relaxed);
        let f = self.failures.load(Ordering::Relaxed);
        let total = s + f;
        let success_rate = if total == 0 { 1.0 } else { s as f32 / total as f32 };
        let last = *self.last_check.lock();
        HealthScore {
            latency_ms: self.recent_latency_ms.load(Ordering::Relaxed),
            success_rate,
            last_check_ok: last.map(|(_, ok)| ok),
            last_check_at: last.and_then(|(i, _)| {
                let d = i.elapsed();
                if d < Duration::from_secs(365 * 24 * 3600) {
                    Some(std::time::SystemTime::now() - d)
                } else {
                    None
                }
            }),
        }
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiplier_penalises_slow_models() {
        let s = HealthScore {
            latency_ms: 20_000,
            success_rate: 1.0,
            last_check_ok: Some(true),
            last_check_at: None,
        };
        assert!(s.multiplier() < 0.5);
    }

    #[test]
    fn multiplier_drops_with_failures() {
        let s = HealthScore {
            latency_ms: 100,
            success_rate: 0.5,
            last_check_ok: Some(false),
            last_check_at: None,
        };
        assert!(s.multiplier() < 0.6);
    }

    #[test]
    fn unknown_is_neutral() {
        let s = HealthScore::unknown();
        assert_eq!(s.multiplier(), 1.0);
    }

    #[test]
    fn record_outcome_updates_snapshot() {
        let h = ModelHealth::new("m1");
        h.record_outcome(true, 200);
        h.record_outcome(false, 5_000);
        let snap = h.snapshot();
        assert_eq!(snap.latency_ms, 5_000);
        assert!((snap.success_rate - 0.5).abs() < 1e-6);
    }
}
