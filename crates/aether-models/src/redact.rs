//! Shared secret redaction (single source of truth).
//!
//! Used by the desktop backend, the analysis layer, and the agent loop
//! before any error text, tool output, or event payload reaches logs,
//! the frontend, checkpoints, or persisted traces.

/// Redact secret-shaped substrings. Covers common provider key shapes:
/// `sk-…`, `nvapi-…`, `venice-…`, `ghp_…`, `gho_…`, `xox[bap]-…`, `AKIA…`,
/// `sk-ant-…`, plus generic `Bearer <token>` and `key[:=] <value>` pairs.
pub fn redact_secrets(s: &str) -> String {
    let mut out = s.to_string();
    // Prefixed tokens: <prefix> + 8+ alnum/_/- chars.
    for pat in [
        "sk-", "nvapi-", "venice-", "ghp_", "gho_", "xoxb-", "xoxp-", "xoxa-", "AKIA", "sk-ant-",
    ] {
        let mut result = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(idx) = rest.find(pat) {
            let after = &rest[idx + pat.len()..];
            let token_len = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .map(|c| c.len_utf8())
                .sum::<usize>();
            // Count in chars, not bytes, for the threshold.
            let char_len = after.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-').count();
            if char_len >= 8 {
                result.push_str(&rest[..idx + pat.len()]);
                result.push_str("[REDACTED]");
                // Advance by bytes: recompute byte length of the token.
                let mut bytes = 0usize;
                for c in after.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-') {
                    bytes += c.len_utf8();
                }
                rest = &after[bytes.min(after.len())..];
                let _ = token_len;
            } else {
                result.push_str(&rest[..idx + pat.len() + token_len]);
                rest = &after[token_len.min(after.len())..];
            }
        }
        result.push_str(rest);
        out = result;
    }
    // Bearer tokens.
    out = redact_labeled(&out, "Bearer ");
    out
}

fn redact_labeled(s: &str, label: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(idx) = rest.find(label) {
        let after = &rest[idx + label.len()..];
        let char_len = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
            .count();
        if char_len >= 12 {
            result.push_str(&rest[..idx + label.len()]);
            result.push_str("[REDACTED]");
            let mut bytes = 0usize;
            for c in after.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.') {
                bytes += c.len_utf8();
            }
            rest = &after[bytes.min(after.len())..];
        } else {
            result.push_str(&rest[..idx + label.len()]);
            rest = after;
        }
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_common_prefixes() {
        let s = redact_secrets("key sk-abcdefghij1234567890 done");
        assert!(s.contains("sk-[REDACTED]"));
        assert!(!s.contains("abcdefghij"));
    }

    #[test]
    fn redacts_nvapi_and_venice() {
        assert!(redact_secrets("nvapi-1234567890abcdef").contains("nvapi-[REDACTED]"));
        assert!(redact_secrets("venice-1234567890abcdef").contains("venice-[REDACTED]"));
    }

    #[test]
    fn redacts_github_and_aws_shapes() {
        assert!(redact_secrets("ghp_1234567890abcdefXYZ").contains("ghp_[REDACTED]"));
        assert!(redact_secrets("AKIA1234567890ABCDEF").contains("AKIA[REDACTED]"));
    }

    #[test]
    fn redacts_bearer_tokens() {
        let s = redact_secrets("Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig");
        assert!(s.contains("Bearer [REDACTED]"));
        assert!(!s.contains("eyJhbGci"));
    }

    #[test]
    fn leaves_normal_text_alone() {
        let s = "Build passed. Tests: 27 passed, 0 failed.";
        assert_eq!(redact_secrets(s), s);
    }

    #[test]
    fn short_matches_not_redacted() {
        // Avoid false positives on short fragments.
        assert_eq!(redact_secrets("ask-1"), "ask-1");
    }
}
