//! Code-analysis tools exposed to the agent's toolset.
//!
//! These tools make the **code analysis capability** visible to the
//! controller (Model 2) and executor (Model 1) through AETHER's regular tool
//! layer — the same permission engine governs them. They return compact,
//! normalised, sanitised output; full findings stay in the persisted
//! [`AnalysisReport`] and can be drilled into via `analysis_status`.
//!
//! Authority chain reminder: findings are advisory. These tools never mutate
//! files and never execute analyzer output.

use std::sync::Arc;

use async_trait::async_trait;
use aether_permissions::Permission;
use serde_json::Value;

use crate::{Tool, ToolContext, ToolError, ToolResult};

/// Cap on how many finding lines a tool result carries so context pressure
/// stays bounded (the context manager handles the rest).
const MAX_INLINE_FINDINGS: usize = 25;

impl From<aether_analysis::store::StoreError> for ToolError {
    fn from(e: aether_analysis::store::StoreError) -> Self {
        ToolError::Other(format!("analysis store: {e}"))
    }
}

fn provider_from_args(args: &Value) -> Result<Box<dyn aether_analysis::AnalysisProvider>, ToolError> {
    // Provider selection: only `sonarqube` is wired for now; the abstraction
    // allows later providers without touching these tools.
    let provider = arg(&args, "provider").unwrap_or_else(|| "sonarqube".to_string());
    match provider.as_str() {
        "sonarqube" => {
            let base_url = arg(&args, "base_url")
                .or_else(|| std::env::var("SONAR_HOST_URL").ok())
                .unwrap_or_else(|| "http://localhost:9000".to_string());
            let token_env = arg(&args, "token_env").unwrap_or_else(|| "SONAR_TOKEN".to_string());
            let mode = match arg(&args, "mode").as_deref() {
                Some("scanner") => aether_analysis::SonarQubeMode::ScannerApi,
                _ => aether_analysis::SonarQubeMode::Api,
            };
            let cfg = aether_analysis::SonarQubeConfig {
                base_url,
                token_env,
                mode,
                ..Default::default()
            };
            Ok(Box::new(aether_analysis::SonarQubeProvider::new(cfg)))
        }
        other => Err(ToolError::Other(format!(
            "unknown analysis provider '{other}' (available: sonarqube)"
        ))),
    }
}

fn arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

fn render_report_summary(report: &aether_analysis::AnalysisReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "ANALYSIS COMPLETE provider={} project={} report={} at={}\n",
        report.provider, report.project, report.id, report.at,
    ));
    let d = &report.distribution;
    out.push_str(&format!(
        "findings={} (blocker={} high={} medium={} low={} info={}) files={}\n",
        d.total(), d.blocker, d.high, d.medium, d.low, d.info,
        report.affected_files.len(),
    ));
    let mut sorted: Vec<&aether_analysis::Finding> = report.findings.iter().collect();
    sorted.sort_by(|a, b| b.severity.cmp(&a.severity).then_with(|| a.location.path.cmp(&b.location.path)));
    let shown = sorted.len().min(MAX_INLINE_FINDINGS);
    if sorted.len() > shown {
        out.push_str(&format!(
            "showing {shown}/{} most severe findings (full report persisted under report id above)\n",
            sorted.len()
        ));
    }
    for f in sorted.iter().take(shown) {
        out.push_str(&format!("{}\n", f.render_line()));
    }
    out
}

/// Tool: run (or fetch) a deterministic code analysis.
pub struct AnalyzeCodeTool;

#[async_trait]
impl Tool for AnalyzeCodeTool {
    fn name(&self) -> &str {
        "analyze_code"
    }
    fn description(&self) -> &str {
        "Run static code analysis (SonarQube) on the project and return normalised \
         findings. Use when the task involves code quality, security review, bug \
         hunting, or verifying that fixes resolved findings. mode='run' fetches \
         current server results; mode='scanner' also launches sonar-scanner first."
    }
    fn category(&self) -> &'static str {
        "read"
    }
    fn required_permission(&self) -> Permission {
        Permission::Allow
    }
    fn json_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "provider": { "type": "string", "description": "analyzer id (default 'sonarqube')" },
                "mode": { "type": "string", "enum": ["run", "scanner"] },
                "base_url": { "type": "string" },
                "token_env": { "type": "string", "description": "env var name holding the token" },
                "scope": { "type": "array", "items": { "type": "string" } },
                "label": { "type": "string" }
            }
        })
    }
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let provider = provider_from_args(&args)?;
        let av = provider.availability().await;
        if !av.available {
            return Ok(ToolResult {
                output: format!("analysis unavailable: {}", av.detail),
                is_error: true,
            });
        }
        let scope: Vec<String> = args
            .get("scope")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let mut req = aether_analysis::AnalysisRequest::new(ctx.cwd.display().to_string());
        req.scope = scope;
        req.label = arg(&args, "label");
        let mode = arg(&args, "mode").unwrap_or_else(|| "run".to_string());
        let result = if mode == "scanner" {
            provider.analyze(&req).await
        } else {
            match provider.latest_findings(&req).await {
                Ok(Some(findings)) => {
                    let mut rep = aether_analysis::AnalysisReport::new(
                        provider.id(),
                        &aether_analysis::project_key(&req.project_root),
                        &req.project_root,
                        findings,
                    );
                    rep.label = req.label.clone();
                    Ok(rep)
                }
                Ok(None) => provider.analyze(&req).await,
                Err(e) => Err(e),
            }
        };
        match result {
            Ok(report) => {
                if let Ok(store) = aether_analysis::AnalysisStore::default_dir() {
                    let _ = store.save(&report);
                }
                Ok(ToolResult {
                    output: render_report_summary(&report),
                    is_error: false,
                })
            }
            Err(e) => Ok(ToolResult {
                output: format!("analysis failed: {e}"),
                is_error: true,
            }),
        }
    }
}

/// Tool: inspect stored analysis reports (status/severity distribution/diff).
pub struct AnalysisStatusTool;

#[async_trait]
impl Tool for AnalysisStatusTool {
    fn name(&self) -> &str {
        "analysis_status"
    }
    fn description(&self) -> &str {
        "Show the latest stored code-analysis report for this project, or diff two \
         runs (e.g. before/after fixes) to see resolved, remaining, introduced and \
         regressed findings."
    }
    fn category(&self) -> &'static str {
        "read"
    }
    fn required_permission(&self) -> Permission {
        Permission::Allow
    }
    fn json_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["latest", "diff"] },
                "baseline_report": { "type": "string", "description": "report id for diff baseline" },
                "current_report": { "type": "string", "description": "report id for diff current (defaults to latest)" }
            }
        })
    }
    async fn execute(&self, args: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let store = aether_analysis::AnalysisStore::default_dir()
            .map_err(|e| ToolError::Other(format!("analysis store unavailable: {e}")))?;
        let action = arg(&args, "action").unwrap_or_else(|| "latest".to_string());
        // Project key comes from the cwd when executed in a workspace; but
        // the tool context cwd differs from the analysis root, so fall back to
        // listing all projects' latest reports.
        let projects = list_known_projects(&store);
        if action == "diff" {
            let baseline_id = arg(&args, "baseline_report")
                .ok_or_else(|| ToolError::Other("diff requires 'baseline_report' id".into()))?;
            let (project, baseline) = find_report(&store, &projects, &baseline_id)?;
            let current = match arg(&args, "current_report") {
                Some(id) => {
                    let (_, rep) = find_report(&store, &[project.clone()], &id)?;
                    rep
                }
                None => match store.latest(&project)? {
                    Some(r) => r,
                    None => return Ok(ToolResult { output: "no current report".into(), is_error: true }),
                },
            };
            let d = aether_analysis::diff(
                &baseline.findings,
                &current.findings,
                &baseline.id,
                &current.id,
            );
            let mut out = d.render();
            out.push_str("\nRemaining findings (top 15):\n");
            let remaining: Vec<&aether_analysis::Finding> = current
                .findings
                .iter()
                .filter(|f| d.remaining.contains(&f.fingerprint()))
                .collect();
            for f in remaining.iter().take(15) {
                out.push_str(&format!("{}\n", f.render_line()));
            }
            return Ok(ToolResult { output: out, is_error: false });
        }

        // "latest" across all known projects.
        let mut out = String::new();
        if projects.is_empty() {
            out.push_str("no analysis reports stored yet");
            return Ok(ToolResult { output: out, is_error: false });
        }
        for p in &projects {
            if let Some(rep) = store.latest(p)? {
                out.push_str(&format!(
                    "[{}] latest report {} at {}\n",
                    p, rep.id, rep.at
                ));
                out.push_str(&render_report_summary(&rep));
            }
        }
        Ok(ToolResult { output: out, is_error: false })
    }
}

fn list_known_projects(store: &aether_analysis::AnalysisStore) -> Vec<String> {
    store.projects()
}

fn find_report(
    store: &aether_analysis::AnalysisStore,
    projects: &[String],
    report_id: &str,
) -> Result<(String, aether_analysis::AnalysisReport), ToolError> {
    for p in projects {
        if let Ok(rep) = store.load(p, report_id) {
            return Ok((p.clone(), rep));
        }
    }
    Err(ToolError::Other(format!("report '{report_id}' not found")))
}

/// Build the analysis toolset (used at CLI/desktop startup).
pub fn analysis_tools() -> Vec<Arc<dyn Tool>> {
    vec![Arc::new(AnalyzeCodeTool), Arc::new(AnalysisStatusTool)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ctx() -> ToolContext {
        ToolContext { cwd: PathBuf::from(".") }
    }

    #[test]
    fn arg_helper() {
        let v = serde_json::json!({"provider": "sonarqube", "token_env": ""});
        assert_eq!(arg(&v, "provider"), Some("sonarqube".into()));
        assert_eq!(arg(&v, "token_env"), None);
        assert_eq!(arg(&v, "missing"), None);
    }

    #[test]
    fn provider_from_args_defaults_to_sonarqube() {
        let provider = provider_from_args(&serde_json::json!({})).unwrap();
        assert_eq!(provider.id(), "sonarqube");
        let bad = provider_from_args(&serde_json::json!({"provider": "eslint"}));
        assert!(bad.is_err());
    }

    #[test]
    fn render_summary_is_bounded() {
        use aether_analysis::finding::{FindingKind, Location, Severity};
        use aether_analysis::report::AnalysisReport;
        let findings: Vec<aether_analysis::Finding> = (0..100)
            .map(|i| aether_analysis::Finding {
                id: format!("k{i}"),
                provider: "sonarqube".into(),
                rule: format!("S{i}"),
                severity: Severity::Medium,
                kind: FindingKind::CodeSmell,
                message: "msg".into(),
                location: Location {
                    path: format!("f{i}.ts"),
                    start_line: 1,
                    end_line: 1,
                    source_context: None,
                },
                status: "OPEN".into(),
                project: "p".into(),
                remediation: None,
                rule_url: None,
            })
            .collect();
        let rep = AnalysisReport::new("sonarqube", "p", "/r", findings);
        let rendered = render_report_summary(&rep);
        let finding_lines = rendered.lines().filter(|l| l.starts_with('[')).count();
        assert_eq!(finding_lines, MAX_INLINE_FINDINGS);
        assert!(rendered.contains("showing 25/100"));
    }

    #[tokio::test]
    async fn analyze_tool_reports_unavailable_provider_gracefully() {
        // Point at a dead endpoint; must NOT panic or return Err, just a
        // graceful "unavailable" string.
        let args = serde_json::json!({
            "provider": "sonarqube",
            "base_url": "http://127.0.0.1:1",
            "token_env": "AETHER_TEST_MISSING_TOKEN"
        });
        let res = AnalyzeCodeTool.execute(args, &ctx()).await.unwrap();
        assert!(res.is_error);
        assert!(res.output.contains("unavailable"));
    }

    #[tokio::test]
    async fn status_diff_requires_baseline_id() {
        // Ensure the store location exists so default_dir succeeds.
        let res = AnalysisStatusTool
            .execute(serde_json::json!({"action": "diff"}), &ctx())
            .await;
        match res {
            Ok(r) => assert!(r.is_error && r.output.contains("baseline_report")),
            Err(ToolError::Other(msg)) => assert!(msg.contains("baseline_report") || msg.contains("store")),
            Err(e) => panic!("unexpected error: {e:?}"),
        }
    }
}
