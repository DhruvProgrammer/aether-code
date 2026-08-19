//! Defensive de-sensitisation of analyzer output.
//!
//! Analyzer messages occasionally echo back tokens, keys, connection strings
//! or file contents. Before findings reach prompts, logs, the UI,
//! checkpoints or observability events they are passed through [`sanitize_text`].

static SECRET_PATTERNS: &[&str] = &[
    "sqp_",     // SonarQube project token
    "squ_",     // SonarQube user token
    "ghp_",     // GitHub personal access token
    "gho_",     // GitHub OAuth token
    "github_pat_",
    "glpat-",   // GitLab personal token
    "xoxb-",    // Slack bot token
    "xoxp-",    // Slack user token
    "sk-",      // OpenAI-style key
    "AKIA",     // AWS access key id
    "AIza",     // Google API key
    "-----BEGIN", // PEM material
];

/// Replace common secret-looking substrings with `<redacted>` and strip
/// control characters. Not a guarantee against every secret — combined with
/// never storing credentials in reports, this is defence in depth.
pub fn sanitize_text(input: &str) -> String {
    let mut out: String = input.chars().filter(|c| !c.is_control() || *c == '\n').collect();
    for pat in SECRET_PATTERNS {
        // Replace the token prefix plus the plausible token body.
        while let Some(idx) = out.find(pat) {
            let start = idx;
            let rest = &out[idx + pat.len()..];
            let body_len = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .count();
            let end = idx + pat.len() + body_len;
            let mut rebuilt = String::with_capacity(out.len());
            rebuilt.push_str(&out[..start]);
            rebuilt.push_str("<redacted>");
            rebuilt.push_str(&out[end..]);
            out = rebuilt;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_sonar_tokens() {
        let s = "failed with sqp_abcdef123456 on project";
        let out = sanitize_text(s);
        assert!(!out.contains("sqp_abcdef"));
        assert!(out.contains("<redacted>"));
    }

    #[test]
    fn strips_github_tokens() {
        let out = sanitize_text("token ghp_AbC123xyz90 leaked");
        assert!(!out.contains("AbC123xyz90"));
        assert!(out.contains("<redacted>"));
    }

    #[test]
    fn strips_aws_keys() {
        let out = sanitize_text("found AKIAIOSFODNN7EXAMPLE");
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn keeps_normal_text() {
        let s = "Cognitive Complexity of functions should not be too high";
        assert_eq!(sanitize_text(s), s);
    }

    #[test]
    fn strips_control_characters() {
        let s = "line1\x00\x07line2\nline3";
        let out = sanitize_text(s);
        assert!(!out.contains('\u{0}'));
        assert!(out.contains("line1line2"));
        assert!(out.contains("line2\nline3"));
    }
}
