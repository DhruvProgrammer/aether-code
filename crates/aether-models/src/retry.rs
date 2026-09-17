//! Retry policy for provider HTTP calls (spec: runtime survival).
//!
//! Design reference: OpenHarness `engine/query.py` ("API Retry with Exponential
//! Backoff"). This is an independent Rust implementation — no upstream code is
//! copied. Behavior ported:
//!
//! - Retry transient failures up to `max_retries` (default 3).
//! - Retryable: HTTP 429 / 500 / 502 / 503 / 504 / 529, plus connect and
//!   timeout errors. Auth failures (401/403) and malformed requests (4xx
//!   other than 429) are never retried.
//! - Delay is exponential (`base_delay * 2^attempt`, capped at `max_delay`)
//!   with up to `jitter_ratio` additional random spread.
//! - A server `retry-after` response header overrides the computed delay
//!   (clamped to `max_delay`).
//!
//! Without this, one transient 429/5xx or dropped connection fails the whole
//! agent turn and — over a long session — the whole task.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Statuses worth retrying. 529 is an Anthropic-overload code carried over
/// because several OpenAI-compatible gateways forward it unchanged.
pub const RETRYABLE_STATUS_CODES: &[u16] = &[429, 500, 502, 503, 504, 529];

/// Retry tuning knobs. Defaults mirror the reference harness.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// How many times a failed request is retried (0 = no retries).
    pub max_retries: u32,
    /// Delay before the first retry; doubled per attempt.
    pub base_delay: Duration,
    /// Upper bound for any single delay, including `retry-after`.
    pub max_delay: Duration,
    /// Extra random spread as a fraction of the delay (0.25 = +0..25%).
    pub jitter_ratio: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            jitter_ratio: 0.25,
        }
    }
}

/// One performed retry, for logging / UI ("retrying in Xs") / diagnostics.
#[derive(Debug, Clone)]
pub struct RetryEvent {
    /// Zero-based attempt that just failed.
    pub attempt: u32,
    pub max_retries: u32,
    pub delay: Duration,
    /// HTTP status when the failure was a non-2xx response.
    pub status: Option<u16>,
    pub message: String,
}

/// True for transient HTTP statuses. Everything else — including auth
/// failures — must surface immediately so misconfiguration is never masked
/// as flakiness.
pub fn is_retryable_status(status: u16) -> bool {
    RETRYABLE_STATUS_CODES.contains(&status)
}

/// True for transient transport failures (connect refused/reset, DNS,
/// timeouts). Builder errors are programming bugs; never retry those.
pub fn is_retryable_error(err: &reqwest::Error) -> bool {
    err.is_timeout() || err.is_connect()
}

/// Parse a `retry-after` header value (delta-seconds form). HTTP-date form
/// is not supported and returns `None` — the exponential delay applies.
pub fn parse_retry_after(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

/// Extract `retry-after` from response headers, if present and parseable.
pub fn retry_after_from_headers(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_retry_after)
}

/// Delay before retrying `attempt` (zero-based), honoring `retry_after` and
/// adding clock-sampled jitter. Pure apart from the clock read.
pub fn retry_delay(
    attempt: u32,
    cfg: &RetryConfig,
    retry_after: Option<Duration>,
) -> Duration {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let sample = (nanos % 1000) as f64 / 1000.0;
    retry_delay_with_jitter(attempt, cfg, retry_after, sample)
}

/// Deterministic core of [`retry_delay`]; `jitter_sample` in [0, 1).
/// Split out so tests do not depend on the clock.
pub fn retry_delay_with_jitter(
    attempt: u32,
    cfg: &RetryConfig,
    retry_after: Option<Duration>,
    jitter_sample: f64,
) -> Duration {
    // Server directive wins, but never beyond our own ceiling.
    if let Some(ra) = retry_after {
        return ra.min(cfg.max_delay);
    }
    let grown = cfg
        .base_delay
        .saturating_mul(1u32 << attempt.min(16))
        .min(cfg.max_delay);
    let sample = jitter_sample.clamp(0.0, 1.0);
    grown + grown.mul_f64(cfg.jitter_ratio.max(0.0) * sample)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> RetryConfig {
        RetryConfig::default()
    }

    #[test]
    fn defaults_match_reference_harness() {
        let c = RetryConfig::default();
        assert_eq!(c.max_retries, 3);
        assert_eq!(c.base_delay, Duration::from_secs(1));
        assert_eq!(c.max_delay, Duration::from_secs(30));
        assert_eq!(c.jitter_ratio, 0.25);
    }

    #[test]
    fn retryable_statuses_cover_transients_only() {
        for s in [429, 500, 502, 503, 504, 529] {
            assert!(is_retryable_status(s), "{s} should be retryable");
        }
        // Auth, client errors and success must never be retried.
        for s in [200, 201, 400, 401, 403, 404, 422] {
            assert!(!is_retryable_status(s), "{s} must not be retried");
        }
    }

    #[test]
    fn backoff_doubles_then_caps() {
        let c = cfg();
        assert_eq!(
            retry_delay_with_jitter(0, &c, None, 0.0),
            Duration::from_secs(1)
        );
        assert_eq!(
            retry_delay_with_jitter(1, &c, None, 0.0),
            Duration::from_secs(2)
        );
        assert_eq!(
            retry_delay_with_jitter(2, &c, None, 0.0),
            Duration::from_secs(4)
        );
        // 2^10 s would exceed the ceiling — clamped to max_delay.
        assert_eq!(
            retry_delay_with_jitter(10, &c, None, 0.0),
            Duration::from_secs(30)
        );
        assert_eq!(
            retry_delay_with_jitter(100, &c, None, 0.0),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn jitter_stays_within_ratio() {
        let c = cfg();
        let base = retry_delay_with_jitter(2, &c, None, 0.0);
        let max_jittered = retry_delay_with_jitter(2, &c, None, 0.999);
        assert_eq!(base, Duration::from_secs(4));
        assert!(max_jittered >= base);
        assert!(max_jittered <= base + base.mul_f64(0.25));
        // Out-of-range samples are clamped, never panic.
        let _ = retry_delay_with_jitter(0, &c, None, f64::NAN);
        let _ = retry_delay_with_jitter(0, &c, None, 99.0);
    }

    #[test]
    fn retry_after_overrides_but_respects_ceiling() {
        let c = cfg();
        assert_eq!(
            retry_delay_with_jitter(0, &c, Some(Duration::from_secs(5)), 0.0),
            Duration::from_secs(5)
        );
        assert_eq!(
            retry_delay_with_jitter(0, &c, Some(Duration::from_secs(300)), 0.0),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn parse_retry_after_accepts_seconds_only() {
        assert_eq!(
            parse_retry_after("120"),
            Some(Duration::from_secs(120))
        );
        assert_eq!(parse_retry_after("  2  "), Some(Duration::from_secs(2)));
        assert_eq!(parse_retry_after("0"), Some(Duration::from_secs(0)));
        assert!(parse_retry_after("").is_none());
        assert!(parse_retry_after("soon").is_none());
        assert!(parse_retry_after("-5").is_none());
        // HTTP-date form is out of scope by design.
        assert!(parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT").is_none());
    }
}
