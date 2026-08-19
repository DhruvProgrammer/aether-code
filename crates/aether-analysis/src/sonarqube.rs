//! SonarQube integration.
//!
//! SonarQube is a **deterministic analyzer**, not an LLM. AETHER exposes it in
//! two cooperating modes behind one [`AnalysisProvider`]:
//!
//! * [`SonarQubeMode::Api`] — talk to a running SonarQube server
//!   (`base_url` + token read from an environment variable, never stored).
//! * [`SonarQubeMode::ScannerApi`] — if a `sonar-scanner` binary is on PATH,
//!   launch it against the project, then read results through the API.
//!
//! Findings are fetched from the public `/api/issues/search` (and
//! `/api/hotspots/search`) endpoints, paginated, then normalised into
//! [`crate::finding::Finding`]. All native severities/types are mapped into
//! AETHER's canonical scale.
//!
//! Safety:
//! * Scanner commands are built as argument vectors — no shell interpolation.
//! * The auth token is read from an env var at call time and only ever sent
//!   as a bearer header; it is never included in reports, logs or finding
//!   text.
//! * Nothing in findings is executed.

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use crate::finding::{Finding, FindingKind, Location, Severity};
use crate::provider::{AnalysisError, AnalysisProvider, AnalysisRequest, Availability};
use crate::report::{project_key, AnalysisReport};
use crate::sanitize::sanitize_text;

/// How the provider reaches SonarQube.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SonarQubeMode {
    /// Use an already-configured server; analysis is assumed to have been run
    /// (by CI or by `sonar-scanner` elsewhere) and we fetch current results.
    Api,
    /// Run the `sonar-scanner` CLI against the project, then fetch results.
    ScannerApi,
}

/// Configuration for the SonarQube provider.
///
/// `api_key_env` names the environment variable holding the token — the token
/// itself is never stored, logged or serialised.
#[derive(Debug, Clone)]
pub struct SonarQubeConfig {
    /// Base URL, e.g. `http://localhost:9000`.
    pub base_url: String,
    /// Environment variable containing the auth token.
    pub token_env: String,
    pub mode: SonarQubeMode,
    /// Scanner binary name (default `sonar-scanner`).
    pub scanner_binary: String,
    /// Per-request / scan timeout in seconds.
    pub timeout_secs: u64,
    /// Max issue pages to fetch (500 issues per page).
    pub max_pages: u32,
}

impl Default for SonarQubeConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:9000".into(),
            token_env: "SONAR_TOKEN".into(),
            mode: SonarQubeMode::Api,
            scanner_binary: "sonar-scanner".into(),
            timeout_secs: 600,
            max_pages: 10,
        }
    }
}

pub struct SonarQubeProvider {
    cfg: SonarQubeConfig,
}

impl SonarQubeProvider {
    pub fn new(cfg: SonarQubeConfig) -> Self {
        Self { cfg }
    }

    fn token(&self) -> Option<String> {
        std::env::var(&self.cfg.token_env).ok().filter(|s| !s.is_empty())
    }

    fn client(&self) -> Result<reqwest::Client, AnalysisError> {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(self.cfg.timeout_secs.min(60).max(5)))
            .build()
            .map_err(|e| AnalysisError::Other(format!("http client: {e}")))
    }

    /// Probe server health via `/api/system/status`.
    async fn server_up(&self) -> Result<bool, AnalysisError> {
        let client = self.client()?;
        let mut req = client
            .get(format!("{}/api/system/status", self.cfg.base_url.trim_end_matches('/')));
        if let Some(t) = self.token() {
            req = req.bearer_auth(t);
        }
        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                let body: serde_json::Value = resp
                    .json()
                    .await
                    .unwrap_or_else(|_| serde_json::json!({"status":"UNKNOWN"}));
                Ok(body.get("status").and_then(|s| s.as_str()) == Some("UP"))
            }
            _ => Ok(false),
        }
    }

    /// Run `sonar-scanner` in the project directory (ScannerApi mode).
    async fn run_scanner(&self, req: &AnalysisRequest, key: &str) -> Result<(), AnalysisError> {
        let root = Path::new(&req.project_root);
        if !root.is_dir() {
            return Err(AnalysisError::Other(format!(
                "project root does not exist: {}",
                req.project_root
            )));
        }
        let scanner = which(&self.cfg.scanner_binary).ok_or_else(|| {
            AnalysisError::Failed(format!(
                "'{}' not found on PATH; install SonarScanner or switch to Api mode",
                self.cfg.scanner_binary
            ))
        })?;

        // Argument vector construction only — never shell-string from input.
        let mut args: Vec<String> = vec![
            format!("-Dsonar.projectKey={key}"),
            format!("-Dsonar.projectBaseDir={}", req.project_root),
            "-Dsonar.sourceEncoding=UTF-8".into(),
        ];
        if !req.scope.is_empty() {
            args.push(format!("-Dsonar.inclusions={}", req.scope.join(",")));
        }
        // Passing the token: prefer the env var so it never appears in the
        // process argument list on multi-user systems.
        let mut cmd = tokio::process::Command::new(&scanner);
        cmd.args(&args).current_dir(root).kill_on_drop(true);
        if let Some(t) = self.token() {
            cmd.env(&self.cfg.token_env, t);
        } else {
            cmd.env(&self.cfg.token_env, "");
        }
        let child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| AnalysisError::Failed(format!("failed to launch scanner: {e}")))?;
        let output = tokio::time::timeout(Duration::from_secs(self.cfg.timeout_secs), child.wait_with_output())
            .await
            .map_err(|_| AnalysisError::Timeout(self.cfg.timeout_secs))?
            .map_err(|e| AnalysisError::Failed(format!("scanner wait: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Scrub anything that could echo credentials back.
            let cleaned = sanitize_text(&redact_token(&stderr, self.token().as_deref()));
            return Err(AnalysisError::Failed(format!(
                "sonar-scanner exited with {:?}: {}",
                output.status.code(),
                cleaned.chars().take(800).collect::<String>()
            )));
        }
        Ok(())
    }

    /// Paginate `/api/issues/search` for a project key.
    async fn fetch_issues(&self, key: &str) -> Result<Vec<Finding>, AnalysisError> {
        let client = self.client()?;
        let base = self.cfg.base_url.trim_end_matches('/');
        let mut out: Vec<Finding> = Vec::new();
        let mut page = 1u32;
        loop {
            if page > self.cfg.max_pages {
                break;
            }
            let mut req = client.get(format!(
                "{base}/api/issues/search?projectKeys={key}&ps=500&p={page}&resolved=false"
            ));
            if let Some(t) = self.token() {
                req = req.bearer_auth(t);
            }
            let resp = req
                .send()
                .await
                .map_err(|e| AnalysisError::Failed(format!("issues request failed: {e}")))?;
            if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(AnalysisError::Failed(
                    "unauthorized — check the token environment variable".into(),
                ));
            }
            if !resp.status().is_success() {
                return Err(AnalysisError::Failed(format!(
                    "issues endpoint returned {}",
                    resp.status()
                )));
            }
            let body: IssuesResponse = resp
                .json()
                .await
                .map_err(|e| AnalysisError::Failed(format!("bad issues payload: {e}")))?;
            for issue in &body.issues {
                out.push(normalize_issue(issue, key));
            }
            let total_pages = body
                .paging
                .as_ref()
                .map(|p| ((p.total as f64) / (p.page_size as f64)).ceil() as u32)
                .unwrap_or(0);
            if page >= total_pages || body.issues.is_empty() {
                break;
            }
            page += 1;
        }
        // Security hotspots live on a separate endpoint; best effort.
        if let Ok(hots) = self.fetch_hotspots(&client, base, key).await {
            out.extend(hots);
        }
        Ok(out)
    }

    async fn fetch_hotspots(
        &self,
        client: &reqwest::Client,
        base: &str,
        key: &str,
    ) -> Result<Vec<Finding>, AnalysisError> {
        let mut req = client.get(format!(
            "{base}/api/hotspots/search?projectKey={key}&ps=500&status=TO_REVIEW"
        ));
        if let Some(t) = self.token() {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await.map_err(|e| AnalysisError::Other(format!("{e}")))?;
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }
        let body: HotspotsResponse = match resp.json().await {
            Ok(b) => b,
            Err(_) => return Ok(Vec::new()),
        };
        Ok(body.hotspots.iter().map(|h| normalize_hotspot(h, key)).collect())
    }
}

#[async_trait]
impl AnalysisProvider for SonarQubeProvider {
    fn id(&self) -> &str {
        "sonarqube"
    }

    fn display_name(&self) -> &str {
        "SonarQube"
    }

    async fn availability(&self) -> Availability {
        let token_present = self.token().is_some();
        match self.cfg.mode {
            SonarQubeMode::ScannerApi if which(&self.cfg.scanner_binary).is_none() => {
                return Availability {
                    provider: "sonarqube".into(),
                    available: false,
                    detail: format!("scanner binary '{}' not found on PATH", self.cfg.scanner_binary),
                };
            }
            _ => {}
        }
        match self.server_up().await {
            Ok(true) => Availability {
                provider: "sonarqube".into(),
                available: true,
                detail: format!(
                    "server UP at {}{}",
                    self.cfg.base_url,
                    if token_present { " (token configured)" } else { " (no token env)" }
                ),
            },
            Ok(false) => Availability {
                provider: "sonarqube".into(),
                available: false,
                detail: format!("server not reachable / not UP at {}", self.cfg.base_url),
            },
            Err(e) => Availability {
                provider: "sonarqube".into(),
                available: false,
                detail: format!("server probe failed: {e}"),
            },
        }
    }

    async fn analyze(&self, req: &AnalysisRequest) -> Result<AnalysisReport, AnalysisError> {
        let key = req
            .project_key
            .clone()
            .unwrap_or_else(|| project_key(&req.project_root));
        if self.cfg.mode == SonarQubeMode::ScannerApi {
            self.run_scanner(req, &key).await?;
        }
        let findings = self.fetch_issues(&key).await?;
        let mut report = AnalysisReport::new(self.id(), &key, &req.project_root, findings);
        report.label = req.label.clone();
        Ok(report)
    }

    async fn latest_findings(
        &self,
        req: &AnalysisRequest,
    ) -> Result<Option<Vec<Finding>>, AnalysisError> {
        let key = req
            .project_key
            .clone()
            .unwrap_or_else(|| project_key(&req.project_root));
        let findings = self.fetch_issues(&key).await?;
        Ok(Some(findings))
    }
}

// ---------------------------------------------------------------------------
// SonarQube wire types (minimal subset; parsed defensively)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct IssuesResponse {
    #[serde(default)]
    issues: Vec<Issue>,
    #[serde(default)]
    paging: Option<Paging>,
}

#[derive(Debug, Deserialize)]
struct Paging {
    #[serde(default)]
    total: u32,
    #[serde(rename = "pageSize", default = "default_ps")]
    page_size: u32,
}
fn default_ps() -> u32 { 500 }

#[derive(Debug, Deserialize)]
struct Issue {
    #[serde(default)]
    key: String,
    #[serde(default)]
    rule: String,
    #[serde(default)]
    severity: String,
    #[serde(rename = "type", default)]
    issue_type: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    component: String,
    #[serde(default)]
    status: String,
    #[serde(rename = "textRange")]
    text_range: Option<TextRange>,
    #[serde(default)]
    effort: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TextRange {
    #[serde(rename = "startLine", default)]
    start_line: u32,
    #[serde(rename = "endLine", default)]
    end_line: u32,
}

#[derive(Debug, Deserialize)]
struct HotspotsResponse {
    #[serde(default)]
    hotspots: Vec<Hotspot>,
}

#[derive(Debug, Deserialize)]
struct Hotspot {
    #[serde(default)]
    key: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    component: String,
    #[serde(default)]
    status: String,
    #[serde(rename = "textRange")]
    text_range: Option<TextRange>,
}

/// Map native SonarQube severity onto AETHER's canonical scale.
pub fn map_severity(s: &str) -> Severity {
    match s.to_ascii_uppercase().as_str() {
        "BLOCKER" => Severity::Blocker,
        "CRITICAL" => Severity::High,
        "MAJOR" => Severity::Medium,
        "MINOR" => Severity::Low,
        _ => Severity::Info,
    }
}

/// Map native issue type onto AETHER's kind.
pub fn map_kind(t: &str) -> FindingKind {
    match t.to_ascii_uppercase().as_str() {
        "BUG" => FindingKind::Bug,
        "VULNERABILITY" => FindingKind::Vulnerability,
        "SECURITY_HOTSPOT" => FindingKind::SecurityHotspot,
        _ => FindingKind::CodeSmell,
    }
}

/// Extract the path portion of a SonarQube component key (`key:src/a.ts`).
/// A bare project component (no `:` or `rest == project`) maps to the
/// project root, represented as an empty path.
fn component_path(component: &str, project: &str) -> String {
    if let Some((_, rest)) = component.split_once(':') {
        if rest == project || rest.is_empty() {
            return String::new();
        }
        rest.to_string()
    } else {
        String::new()
    }
}

/// Pure, testable normalisation of one SonarQube issue into an AETHER finding.
fn normalize_issue(issue: &Issue, project: &str) -> Finding {
    let path = component_path(&issue.component, project);
    let (start, end) = issue
        .text_range
        .as_ref()
        .map(|r| (r.start_line.max(1), r.end_line.max(r.start_line.max(1))))
        .unwrap_or((1, 1));
    let rule_display = issue.rule.clone();
    Finding {
        id: issue.key.clone(),
        provider: "sonarqube".into(),
        rule: rule_display,
        severity: map_severity(&issue.severity),
        kind: map_kind(&issue.issue_type),
        message: sanitize_text(&issue.message),
        location: Location {
            path,
            start_line: start,
            end_line: end,
            source_context: None,
        },
        status: issue.status.clone(),
        project: project.into(),
        remediation: issue.effort.clone(),
        rule_url: None,
    }
}

fn normalize_hotspot(h: &Hotspot, project: &str) -> Finding {
    let path = component_path(&h.component, project);
    let (start, end) = h
        .text_range
        .as_ref()
        .map(|r| (r.start_line.max(1), r.end_line.max(r.start_line.max(1))))
        .unwrap_or((1, 1));
    Finding {
        id: h.key.clone(),
        provider: "sonarqube".into(),
        rule: "security-hotspot".into(),
        severity: Severity::Medium,
        kind: FindingKind::SecurityHotspot,
        message: sanitize_text(&h.message),
        location: Location { path, start_line: start, end_line: end, source_context: None },
        status: h.status.clone(),
        project: project.into(),
        remediation: None,
        rule_url: None,
    }
}

/// Parse a raw `/api/issues/search` JSON payload into findings (testing /
/// offline replay without a live server).
pub fn parse_issues_json(json: &str, project: &str) -> Result<Vec<Finding>, AnalysisError> {
    let body: IssuesResponse =
        serde_json::from_str(json).map_err(|e| AnalysisError::Failed(format!("bad payload: {e}")))?;
    Ok(body.issues.iter().map(|i| normalize_issue(i, project)).collect())
}

/// Remove the literal token from any log-like text (defence in depth).
fn redact_token(text: &str, token: Option<&str>) -> String {
    match token {
        Some(t) if !t.is_empty() => text.replace(t, "<redacted>"),
        _ => text.to_string(),
    }
}

/// Minimal `which` without an extra dependency.
fn which(bin: &str) -> Option<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let exts: Vec<&str> = if cfg!(windows) { vec!["", ".exe", ".cmd", ".bat"] } else { vec![""] };
    for dir in std::env::split_paths(&path_var) {
        for ext in &exts {
            let candidate = dir.join(format!("{bin}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_mapping_matches_sonar_scale() {
        assert_eq!(map_severity("BLOCKER"), Severity::Blocker);
        assert_eq!(map_severity("CRITICAL"), Severity::High);
        assert_eq!(map_severity("MAJOR"), Severity::Medium);
        assert_eq!(map_severity("MINOR"), Severity::Low);
        assert_eq!(map_severity("INFO"), Severity::Info);
        assert_eq!(map_severity("weird"), Severity::Info);
    }

    #[test]
    fn kind_mapping() {
        assert_eq!(map_kind("BUG"), FindingKind::Bug);
        assert_eq!(map_kind("VULNERABILITY"), FindingKind::Vulnerability);
        assert_eq!(map_kind("SECURITY_HOTSPOT"), FindingKind::SecurityHotspot);
        assert_eq!(map_kind("CODE_SMELL"), FindingKind::CodeSmell);
        assert_eq!(map_kind("???"), FindingKind::CodeSmell);
    }

    #[test]
    fn parse_issues_json_normalizes_full_payload() {
        let payload = r#"{
            "paging": {"pageIndex": 1, "pageSize": 500, "total": 2},
            "issues": [
                {
                    "key": "AY1",
                    "rule": "typescript:S3776",
                    "severity": "CRITICAL",
                    "type": "CODE_SMELL",
                    "message": "Cognitive Complexity of functions should not be too high",
                    "component": "myproj:src/auth.ts",
                    "status": "OPEN",
                    "textRange": {"startLine": 12, "endLine": 40},
                    "effort": "22min"
                },
                {
                    "key": "AY2",
                    "rule": "secrets:S6290",
                    "severity": "BLOCKER",
                    "type": "VULNERABILITY",
                    "message": "Hard-coded credentials detected",
                    "component": "myproj",
                    "status": "CONFIRMED"
                }
            ]
        }"#;
        let findings = parse_issues_json(payload, "myproj").unwrap();
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].severity, Severity::High);
        assert_eq!(findings[0].kind, FindingKind::CodeSmell);
        assert_eq!(findings[0].location.path, "src/auth.ts");
        assert_eq!(findings[0].location.start_line, 12);
        assert_eq!(findings[0].remediation.as_deref(), Some("22min"));
        assert_eq!(findings[1].severity, Severity::Blocker);
        assert_eq!(findings[1].kind, FindingKind::Vulnerability);
        // Project-root finding gets empty path.
        assert_eq!(findings[1].location.path, "");
        assert_eq!(findings[1].location.start_line, 1);
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse_issues_json("not json", "p").is_err());
    }

    #[test]
    fn token_redaction() {
        let text = "error: auth failed for token squ_abc123xyz";
        let out = redact_token(text, Some("squ_abc123xyz"));
        assert!(!out.contains("squ_abc123xyz"));
        assert!(out.contains("<redacted>"));
        assert_eq!(redact_token(text, None), text);
    }

    #[test]
    fn component_path_extraction() {
        assert_eq!(component_path("myproj:src/a.ts", "myproj"), "src/a.ts");
        assert_eq!(component_path("myproj", "myproj"), "");
    }

    #[tokio::test]
    async fn availability_reports_no_server() {
        let prov = SonarQubeProvider::new(SonarQubeConfig {
            base_url: "http://127.0.0.1:1".into(),
            timeout_secs: 5,
            ..Default::default()
        });
        let av = prov.availability().await;
        assert!(!av.available);
        assert_eq!(av.provider, "sonarqube");
    }
}
