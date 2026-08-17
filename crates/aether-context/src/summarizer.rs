//! Pluggable summarisation strategies.

use async_trait::async_trait;

#[async_trait]
pub trait Summarizer: Send + Sync {
    /// Produce a short summary of `body`. The returned string MUST be much
    /// shorter than `body` (target: ≤ 30% length) and MUST preserve any
    /// structured information the compactor deems critical (errors,
    /// file paths, identifiers, numbers).
    async fn summarize(&self, body: &str, hint: &str) -> String;
}

/// Trivial no-op summariser. Used in tests and as a fallback.
pub struct NoopSummarizer;

#[async_trait]
impl Summarizer for NoopSummarizer {
    async fn summarize(&self, body: &str, _hint: &str) -> String {
        body.to_string()
    }
}

/// Cheap extractive summariser. Picks the first and last sentence, plus a few
/// middle sentences containing high-signal tokens. No external dependency.
pub struct ExtractiveSummarizer {
    pub max_chars: usize,
}

impl Default for ExtractiveSummarizer {
    fn default() -> Self { Self { max_chars: 800 } }
}

#[async_trait]
impl Summarizer for ExtractiveSummarizer {
    async fn summarize(&self, body: &str, hint: &str) -> String {
        // Split on sentence-ish boundaries.
        let sentences: Vec<&str> = body
            .split(|c: char| c == '.' || c == '\n')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if sentences.is_empty() { return String::new(); }
        let hint_lower = hint.to_lowercase();
        let tokens: Vec<&str> = hint_lower.split_whitespace().collect();

        let mut keep: Vec<&str> = Vec::new();
        if let Some(first) = sentences.first() { keep.push(*first); }
        for s in &sentences {
            if keep.len() >= 6 { break; }
            let s_lower = s.to_lowercase();
            if tokens.iter().any(|t| s_lower.contains(t)) {
                if !keep.contains(s) { keep.push(*s); }
            }
        }
        if let Some(last) = sentences.last() {
            if !keep.contains(last) { keep.push(*last); }
        }

        let mut out = keep.join(". ");
        if out.len() > self.max_chars {
            out.truncate(self.max_chars);
            out.push('…');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn extractive_keeps_first_and_last() {
        let s = ExtractiveSummarizer::default();
        let body = "First sentence.\nMiddle sentence about database.\nLast sentence about error.";
        let out = s.summarize(body, "error").await;
        assert!(out.contains("First"));
        assert!(out.contains("Last"));
    }

    #[tokio::test]
    async fn noop_returns_input() {
        let s = NoopSummarizer;
        let body = "hello";
        assert_eq!(s.summarize(body, "").await, "hello");
    }
}
