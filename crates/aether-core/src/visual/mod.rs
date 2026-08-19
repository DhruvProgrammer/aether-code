//! Three-LLM visual engineering subsystem (spec: 3-LLM Visual Engineering Architecture).
//!
//! Roles (kept strictly separated):
//!   * LLM 1 — BIG EXECUTOR (`= hands`): implements corrections. In this codebase that is the
//!     existing `executor` model, surfaced here through [`CorrectionExecutor`].
//!   * LLM 2 — SMALL CONTROLLER (`= brain`): produces the correction plan from LLM 3 evidence.
//!     In this codebase that is the existing `controller` model.
//!   * LLM 3 — VISUAL FRONTEND REVIEWER (`= eyes`): a multimodal model that critiques the
//!     *rendered* website and returns structured evidence. It NEVER commands LLM 1 directly.
//!
//! Communication boundary is always:  LLM 3 → VisualEvidence → LLM 2 → CorrectionPlan → LLM 1.
//!
//! The subsystem is fully optional. If `reviewer_model` is unset, no `capture_command` is
//! configured, or the task is not a frontend task, it degrades gracefully and the normal
//! frontend build continues. Screenshots live only in `~/.aether/temp-screenshots/<exec_id>/`
//! (never inside the project repo) and are removed by a RAII guard on every exit path.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use aether_config::{FrontendConfig, VisualAcceptanceConfig};
use aether_models::{CompletionRequest, Message, ModelProvider};
use aether_sessions::SessionStore;

// ---------------------------------------------------------------------------
// Roles
// ---------------------------------------------------------------------------

/// The three independent LLM roles. LLM 1 (executor) and LLM 2 (controller) already exist in
/// the two-LLM core; this module adds LLM 3 (visual reviewer). The enum documents the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LLMRole {
    /// BIG EXECUTOR — implements. Mandatory.
    Llm1Executor,
    /// SMALL CONTROLLER — plans/routes/decides. Optional for this subsystem.
    Llm2Controller,
    /// VISUAL FRONTEND REVIEWER — sees/critiques/approves. Optional.
    Llm3Reviewer,
}

/// Declarative description of a role's model configuration. Provider/model independence: the user
/// supplies any supported provider + model per role via the existing `models` map.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LLMConfiguration {
    pub enabled: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub api_key_env: Option<String>,
}

// ---------------------------------------------------------------------------
// Review data model
// ---------------------------------------------------------------------------

/// A single visual defect found by LLM 3.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VisualIssue {
    pub severity: String,
    pub category: String,
    pub component: String,
    pub description: String,
    pub recommendation: String,
    /// Optional file the executor should touch to fix this issue (minimal context only).
    #[serde(default)]
    pub relevant_file: Option<String>,
}

/// Structured evidence produced by LLM 3 (never free-form prose).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VisualReviewResult {
    /// `"approved"` | `"rejected"`.
    pub status: String,
    /// 0-100 supporting score (NOT the only approval mechanism).
    #[serde(default)]
    pub score: u32,
    #[serde(default)]
    pub issues: Vec<VisualIssue>,
}

impl VisualReviewResult {
    pub fn is_approved(&self) -> bool {
        self.status.eq_ignore_ascii_case("approved")
    }
}

/// Minimal context sent to LLM 3. Deliberately small: no repo dump, no full conversation.
#[derive(Debug, Clone)]
pub struct VisualReviewContext {
    pub user_requirements: String,
    pub design_requirements: String,
    pub screenshot: Screenshot,
    pub viewport: String,
    pub previous_evidence: String,
}

/// A captured screenshot artifact (temporary only).
#[derive(Debug, Clone)]
pub struct Screenshot {
    pub path: PathBuf,
    /// base64 `data:` URL handed to a vision model.
    pub data_url: String,
    pub viewport: String,
}

// ---------------------------------------------------------------------------
// Explicit acceptance policy (spec §12) — score is supporting evidence only.
// ---------------------------------------------------------------------------

pub struct VisualReviewPolicy;

impl VisualReviewPolicy {
    /// True only when the acceptance contract is satisfied.
    pub fn evaluate(result: &VisualReviewResult, acc: &VisualAcceptanceConfig) -> bool {
        if !result.is_approved() {
            return false;
        }
        if acc.require_no_critical
            && result.issues.iter().any(|i| i.severity.eq_ignore_ascii_case("critical"))
        {
            return false;
        }
        if acc.require_no_major && result.issues.iter().any(|i| i.severity.eq_ignore_ascii_case("major")) {
            return false;
        }
        if let Some(min) = acc.min_score {
            if result.score < min {
                return false;
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// State machine (spec §18) — explicit, resumable.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VisualReviewState {
    #[default]
    Idle,
    FrontendTask,
    Planning,
    Implementation,
    FrontendReady,
    CreatingScreenshot,
    VisualReview,
    VisualRejected,
    CorrectionPlanning,
    CorrectionImplementation,
    VisualApproved,
    VisualEscalated,
    Cleanup,
    Complete,
}

impl VisualReviewState {
    pub fn label(&self) -> &'static str {
        match self {
            VisualReviewState::Idle => "IDLE",
            VisualReviewState::FrontendTask => "FRONTEND_TASK",
            VisualReviewState::Planning => "PLANNING",
            VisualReviewState::Implementation => "IMPLEMENTATION",
            VisualReviewState::FrontendReady => "FRONTEND_READY",
            VisualReviewState::CreatingScreenshot => "CREATING_SCREENSHOT",
            VisualReviewState::VisualReview => "VISUAL_REVIEW",
            VisualReviewState::VisualRejected => "VISUAL_REJECTED",
            VisualReviewState::CorrectionPlanning => "CORRECTION_PLANNING",
            VisualReviewState::CorrectionImplementation => "CORRECTION_IMPLEMENTATION",
            VisualReviewState::VisualApproved => "VISUAL_APPROVED",
            VisualReviewState::VisualEscalated => "VISUAL_ESCALATED",
            VisualReviewState::Cleanup => "CLEANUP",
            VisualReviewState::Complete => "COMPLETE",
        }
    }
}

// ---------------------------------------------------------------------------
// Gate — decide whether to run the visual loop at all (spec §5, graceful degrade).
// ---------------------------------------------------------------------------

/// True only when LLM 3 is fully configured AND (the task is a frontend task OR `force` is set).
pub fn should_run_visual_review(task: &str, reviewer_model: &Option<String>, frontend: &FrontendConfig) -> bool {
    let configured = reviewer_model.is_some()
        && frontend.max_visual_iterations > 0
        && frontend.capture_command.is_some();
    if !configured {
        return false;
    }
    if frontend.force {
        return true;
    }
    is_frontend_task(task)
}

/// Heuristic frontend-task detection (spec §5). LLM 2 may also plan frontend work normally;
/// this only gates *whether LLM 3 is invoked*.
pub fn is_frontend_task(task: &str) -> bool {
    let t = task.to_ascii_lowercase();
    let signals = [
        "website",
        "web site",
        "landing page",
        "webpage",
        "web page",
        "frontend",
        "front-end",
        "ui/ux",
        "ui ux",
        "dashboard",
        "saas",
        "html",
        "css",
        "responsive",
        "design me",
        "design a",
        "build a website",
        "build a beautiful",
        "build a modern",
        "create a website",
        "create a landing",
        "make the ui",
        "make the ux",
        "premium",
        "beautiful website",
        "web app",
        "component library",
        "visual",
    ];
    signals.iter().any(|s| t.contains(s))
}

// ---------------------------------------------------------------------------
// Temporary screenshot workspace (spec §7, §15, §16, §17) — RAII guaranteed cleanup.
// ---------------------------------------------------------------------------

pub struct TempScreenshotWorkspace {
    root: PathBuf,
    preview_child: Option<tokio::process::Child>,
}

impl TempScreenshotWorkspace {
    /// Create `temp-screenshots/<exec_id>/` under the aether data dir (outside the repo).
    fn create(exec_id: &str) -> std::io::Result<Self> {
        let root = aether_config::Config::default_dir().join("temp-screenshots").join(exec_id);
        std::fs::create_dir_all(&root)?;
        Ok(Self { root, preview_child: None })
    }

    /// Capture the rendered frontend. Starts `preview_command` (background) if set, then runs
    /// `capture_command` (with `{out}` and `{cwd}` substituted), reads the PNG and returns it as
    /// a base64 `data:` URL. Returns an error (so the loop is skipped) when no capture command
    /// is configured.
    async fn capture(
        &mut self,
        cwd: &Path,
        capture_command: &str,
        preview_command: &Option<String>,
    ) -> Result<Screenshot> {
        // Defense in depth: the `{cwd}` and `{out}` tokens are substituted into a shell string
        // (`cmd /C` / `sh -c`). Escape both values for the active shell so a path containing
        // spaces, `&`, `()`, etc. (e.g. `C:\Program Files (x86)\…`) cannot inject commands.
        // The `capture_command` / `preview_command` strings themselves are user-controlled
        // (config-trusted) — same trust model as the `bash` tool.
        let cwd_esc = shell_escape(cwd);
        let out = self.root.join("shot.png");
        let out_esc = shell_escape(&out);

        // Optional preview server, killed on cleanup. Sleep is conditional: only when the
        // spawn actually produced a child do we wait for the server to come up.
        if let Some(preview) = preview_command {
            let cmd = preview.replace("{cwd}", &cwd_esc);
            let (shell, flag) = shell_pair();
            if let Ok(child) = tokio::process::Command::new(shell)
                .arg(flag)
                .arg(&cmd)
                .current_dir(cwd)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                self.preview_child = Some(child);
                // Give the server a moment to come up.
                tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            }
        }

        let cmd = capture_command
            .replace("{out}", &out_esc)
            .replace("{cwd}", &cwd_esc);
        let (shell, flag) = shell_pair();
        let status = tokio::process::Command::new(shell)
            .arg(flag)
            .arg(&cmd)
            .current_dir(cwd)
            .output()
            .await?;
        if !status.status.success() {
            anyhow::bail!(
                "screenshot capture command failed (exit {}): {}",
                status.status.code().unwrap_or(-1),
                String::from_utf8_lossy(&status.stderr)
            );
        }
        if !out.exists() {
            anyhow::bail!("screenshot capture command did not produce {}", out.display());
        }
        let bytes = tokio::fs::read(&out).await?;
        // base64 is synchronous and CPU-bound — offload so we don't stall the runtime.
        let encoded = tokio::task::spawn_blocking(move || {
            base64::engine::general_purpose::STANDARD.encode(bytes)
        })
        .await?;
        Ok(Screenshot {
            path: out,
            data_url: format!("data:image/png;base64,{encoded}"),
            viewport: "desktop".to_string(),
        })
    }

    fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Drop for TempScreenshotWorkspace {
    fn drop(&mut self) {
        if let Some(mut c) = self.preview_child.take() {
            let _ = c.start_kill();
            // Best-effort reap so the preview server does not linger as a zombie on Unix.
            // Non-blocking: if the process is still tearing down, the OS reaps it on parent exit.
            let _ = c.try_wait();
        }
        self.cleanup();
    }
}

/// Remove a specific execution's screenshot workspace regardless of how we exit (spec §17).
/// Only the per-execution subdirectory is removed; the parent `temp-screenshots/` is left in
/// place so concurrent sessions cannot delete each other's roots.
pub fn cleanup_workspace(exec_id: &str) {
    let root = aether_config::Config::default_dir().join("temp-screenshots").join(exec_id);
    let _ = std::fs::remove_dir_all(&root);
}

fn shell_pair() -> (&'static str, &'static str) {
    if cfg!(target_os = "windows") {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    }
}

/// Escape a path for safe substitution into a `cmd /C` or `sh -c` string. The result is
/// already wrapped in the appropriate quotes so a path containing spaces, `&`, `()`, etc.
/// (e.g. `C:\Program Files (x86)\…`) is passed to the program as a single literal argument
/// rather than being reinterpreted by the shell.
fn shell_escape(p: &Path) -> String {
    let s = p.to_string_lossy();
    if cfg!(target_os = "windows") {
        // cmd.exe: wrap in double quotes; escape any embedded `"` by doubling it.
        let escaped = s.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        // sh: wrap in single quotes; a `'` inside becomes `'\''` (close, escaped, reopen).
        let escaped = s.replace('\'', "'\\''");
        format!("'{escaped}'")
    }
}

// ---------------------------------------------------------------------------
// Correction executor — LLM 1 boundary (spec §11). LLM 3 never calls this.
// ---------------------------------------------------------------------------

/// Implemented by the existing agent core (LLM 1 / BIG EXECUTOR). The visual engine asks LLM 2
/// for a correction plan, then calls this to actually implement it.
///
/// NOTE: not bound to `Send + Sync` — the implementor (`Agent`) carries a `SessionStore` whose
/// inner `rusqlite` handle is `!Sync`. The visual loop runs in the same task as the caller, so
/// the `?Send` bound is sufficient and avoids forcing a `Sync` session store.
#[async_trait(?Send)]
pub trait CorrectionExecutor {
    async fn implement_correction(&self, plan: &str) -> Result<String>;
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

/// Outcome of a visual-review run, surfaced to the caller and persisted for observability.
#[derive(Debug, Clone, Default)]
pub struct VisualReviewReport {
    pub state: VisualReviewState,
    pub approved: bool,
    pub escalated: bool,
    /// True when the loop never ran (not configured / not a frontend task / no capture).
    pub skipped: bool,
    pub iterations: u32,
    pub last: Option<VisualReviewResult>,
    pub summary: String,
}

pub struct VisualReviewEngine {
    reviewer: Arc<dyn ModelProvider>,
    reviewer_model: String,
    controller: Arc<dyn ModelProvider>,
    controller_model: String,
    frontend: FrontendConfig,
    cwd: PathBuf,
    session: Option<Arc<SessionStore>>,
    session_id: String,
}

impl VisualReviewEngine {
    pub fn new(
        reviewer: Arc<dyn ModelProvider>,
        reviewer_model: String,
        controller: Arc<dyn ModelProvider>,
        controller_model: String,
        frontend: FrontendConfig,
        cwd: PathBuf,
        session: Option<Arc<SessionStore>>,
        session_id: String,
    ) -> Self {
        Self {
            reviewer,
            reviewer_model,
            controller,
            controller_model,
            frontend,
            cwd,
            session,
            session_id,
        }
    }

    fn trace(&self, kind: &str, agent: &str, summary: &str, payload: &str) {
        if let Some(s) = &self.session {
            let _ = s.record_trace(&self.session_id, kind, agent, None, summary, payload);
        }
    }

    fn persist_state(&self, state: VisualReviewState) {
        if let Some(s) = &self.session {
            let _ = s.set_kv(&self.session_id, "visual_state", state.label());
        }
    }

    /// Run the visual engineering loop (spec §10). Returns a report; never hard-fails the task —
    /// on any error it reports a skipped/escalated outcome so the caller can continue.
    pub async fn run(
        &self,
        task: &str,
        design_context: &str,
        executor: &dyn CorrectionExecutor,
    ) -> VisualReviewReport {
        // Resumability: if a previous run already approved this session, don't re-review.
        if let Some(s) = &self.session {
            if let Ok(Some(v)) = s.get_kv(&self.session_id, "visual_state") {
                if v == VisualReviewState::VisualApproved.label() {
                    return VisualReviewReport {
                        state: VisualReviewState::VisualApproved,
                        approved: true,
                        summary: "Visual review already approved in a prior run.".into(),
                        ..Default::default()
                    };
                }
            }
        }

        let capture = match &self.frontend.capture_command {
            Some(c) => c.clone(),
            None => {
                self.trace("visual_gate", "llm3-reviewer", "skipped: no capture_command", "");
                return VisualReviewReport {
                    state: VisualReviewState::Complete,
                    skipped: true,
                    summary: "Visual review skipped: no capture_command configured.".into(),
                    ..Default::default()
                };
            }
        };

        self.persist_state(VisualReviewState::FrontendReady);
        self.trace("visual_ready", "loop", "FRONTEND_READY — entering visual review", "");

        // RAII: the workspace (and any preview server) is torn down on every return path.
        let mut workspace = match TempScreenshotWorkspace::create(&self.session_id) {
            Ok(w) => w,
            Err(e) => {
                return VisualReviewReport {
                    state: VisualReviewState::VisualEscalated,
                    escalated: true,
                    summary: format!("Visual review failed to create workspace: {e}"),
                    ..Default::default()
                };
            }
        };

        let max_iter = self.frontend.max_visual_iterations as usize;
        let mut prev_signature: Option<String> = None;
        let mut repeat_count: usize = 0;
        let mut scores: Vec<u32> = Vec::new();
        let mut last: Option<VisualReviewResult> = None;

        for iter in 0..max_iter {
            // --- Capture -----------------------------------------------------
            self.persist_state(VisualReviewState::CreatingScreenshot);
            let screenshot = match workspace
                .capture(&self.cwd, &capture, &self.frontend.preview_command)
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    self.persist_state(VisualReviewState::VisualEscalated);
                    self.trace("visual_capture", "loop", &format!("capture failed: {e}"), "");
                    return VisualReviewReport {
                        state: VisualReviewState::VisualEscalated,
                        escalated: true,
                        summary: format!("Screenshot capture failed: {e}"),
                        ..Default::default()
                    };
                }
            };
            self.trace(
                "visual_capture",
                "loop",
                &format!("captured {} (iter {})", screenshot.path.display(), iter + 1),
                "",
            );

            // --- LLM 3 review -------------------------------------------------
            self.persist_state(VisualReviewState::VisualReview);
            let ctx = VisualReviewContext {
                user_requirements: task.to_string(),
                design_requirements: design_context.to_string(),
                screenshot,
                viewport: "desktop".to_string(),
                previous_evidence: last
                    .as_ref()
                    .map(|r| format!("status={} score={} issues={}", r.status, r.score, issues_brief(r)))
                    .unwrap_or_default(),
            };
            let result = self.review(&ctx).await;
            let result = match result {
                Ok(r) => r,
                Err(e) => {
                    self.trace("visual_review", "llm3-reviewer", &format!("review error: {e}"), "");
                    let fallback = VisualReviewResult {
                        status: "rejected".into(),
                        score: 0,
                        issues: vec![VisualIssue {
                            severity: "major".into(),
                            category: "review".into(),
                            component: "review".into(),
                            description: format!("LLM 3 review failed to parse: {e}"),
                            recommendation: "Retry visual review.".into(),
                            relevant_file: None,
                        }],
                    };
                    fallback
                }
            };
            scores.push(result.score);
            self.trace(
                "visual_review",
                "llm3-reviewer",
                &format!(
                    "status={} score={} issues={}",
                    result.status,
                    result.score,
                    result.issues.len()
                ),
                &serde_json::to_string(&result).unwrap_or_default(),
            );

            // --- Acceptance gate -------------------------------------------
            if VisualReviewPolicy::evaluate(&result, &self.frontend.acceptance) {
                self.persist_state(VisualReviewState::VisualApproved);
                self.trace("visual_approve", "llm3-reviewer", "APPROVED", "");
                cleanup_workspace(&self.session_id);
                return VisualReviewReport {
                    state: VisualReviewState::VisualApproved,
                    approved: true,
                    iterations: (iter + 1) as u32,
                    last: Some(result),
                    summary: format!(
                        "Visual review APPROVED after {} iteration(s) (score {}).",
                        iter + 1,
                        scores.last().copied().unwrap_or(0)
                    ),
                    ..Default::default()
                };
            }

            // --- Rejected: loop protection ----------------------------------
            self.persist_state(VisualReviewState::VisualRejected);
            let sig = issue_signature(&result);
            if prev_signature.as_deref() == Some(&sig) {
                repeat_count += 1;
            } else {
                repeat_count = 0;
                prev_signature = Some(sig);
            }
            let flat = scores.len() >= 3
                && (scores.iter().max().copied().unwrap_or(0) - scores.iter().min().copied().unwrap_or(0)) <= 1;
            if repeat_count >= 2 || flat {
                self.persist_state(VisualReviewState::VisualEscalated);
                self.trace(
                    "visual_escalate",
                    "loop",
                    "escalated: repeated/identical feedback or flat score",
                    &serde_json::to_string(&result).unwrap_or_default(),
                );
                cleanup_workspace(&self.session_id);
                return VisualReviewReport {
                    state: VisualReviewState::VisualEscalated,
                    escalated: true,
                    iterations: (iter + 1) as u32,
                    last: Some(result.clone()),
                    summary: format!(
                        "Visual review ESCALATED after {} iteration(s): LLM 3 could not reach the \
                         acceptance policy within limits. Last evidence: status={} score={} issues={}",
                        iter + 1,
                        result.status,
                        result.score,
                        issues_brief(&result)
                    ),
                    ..Default::default()
                };
            }

            // --- LLM 2 correction plan (spec §11) --------------------------
            self.persist_state(VisualReviewState::CorrectionPlanning);
            let plan = self.correction_plan(task, &result, design_context).await;
            self.trace("visual_correct_plan", "llm2-controller", "correction plan", &plan);

            // --- LLM 1 implements (spec §11) -------------------------------
            self.persist_state(VisualReviewState::CorrectionImplementation);
            match executor.implement_correction(&plan).await {
                Ok(out) => {
                    self.trace(
                        "visual_correct_impl",
                        "llm1-executor",
                        "implemented correction",
                        &out.chars().take(500).collect::<String>(),
                    );
                }
                Err(e) => {
                    self.trace("visual_correct_impl", "llm1-executor", &format!("correction error: {e}"), "");
                }
            }
            last = Some(result);
        }

        // Loop exhausted without approval.
        self.persist_state(VisualReviewState::VisualEscalated);
        cleanup_workspace(&self.session_id);
        VisualReviewReport {
            state: VisualReviewState::VisualEscalated,
            escalated: true,
            iterations: max_iter as u32,
            last: last.clone(),
            summary: format!(
                "Visual review ESCALATED: reached max {} iterations without meeting the acceptance \
                 policy. Last evidence: {}",
                max_iter,
                last.as_ref().map(issues_brief).unwrap_or_default()
            ),
            ..Default::default()
        }
    }

    /// Call LLM 3 (vision) with the screenshot and return structured evidence.
    async fn review(&self, ctx: &VisualReviewContext) -> Result<VisualReviewResult> {
        let system = format!(
            "{core}\n\n---\n\nYou are the Visual Frontend Reviewer (LLM 3 / EYES). You evaluate the ACTUAL \
            rendered website shown in the provided screenshot. Assess visual hierarchy, layout, \
            spacing, typography, colors, component consistency, responsive/mobile behavior, UX, \
            accessibility-related visual issues, and adherence to the user's design direction. \
            You MUST reply with ONLY a single JSON object and no prose: \
            {{\"status\":\"approved\"|\"rejected\",\"score\":<0-100>,\"issues\":\
            [{{\"severity\":\"critical\"|\"major\"|\"minor\",\"category\":string,\"component\":string,\
            \"description\":string,\"recommendation\":string}}]}}.",
            core = crate::prompt::AETHER_CORE_SYSTEM_PROMPT,
        );
        let user = format!(
            "USER FRONTEND REQUIREMENTS:\n{}\n\nDESIGN REQUIREMENTS:\n{}\n\nVIEWPORT: {}\n\n\
            PREVIOUS REVIEW EVIDENCE:\n{}\n\nEvaluate the screenshot and return the JSON review.",
            ctx.user_requirements, ctx.design_requirements, ctx.viewport, ctx.previous_evidence
        );
        let req = CompletionRequest {
            model: self.reviewer_model.clone(),
            messages: vec![
                Message { role: "system".into(), content: system.into(), ..Default::default() },
                Message { role: "user".into(), content: user, ..Default::default() },
            ],
            images: Some(vec![ctx.screenshot.data_url.clone()]),
            ..Default::default()
        };
        let resp = self.reviewer.complete(req).await?;
        let text = resp.content.unwrap_or_default();
        Ok(parse_review(&text))
    }

    /// LLM 2 turns LLM 3's evidence into a correction plan (LLM 3 never commands LLM 1 directly).
    async fn correction_plan(&self, task: &str, result: &VisualReviewResult, design_context: &str) -> String {
        let issues_json = serde_json::to_string(&result.issues).unwrap_or_default();
        let prompt = format!(
            "A visual frontend reviewer (LLM 3) rejected the current implementation of this task:\n\
            \nTASK: {task}\n\nDESIGN CONTEXT:\n{design_context}\n\nREVIEW EVIDENCE (status={}, score={}):\n\
            {issues_json}\n\nProduce a CONCRETE, ordered correction plan that an implementer can execute \
            via code edits to fix the listed issues. Reference specific files/components where possible. \
            Output only the correction plan, no preamble.",
            result.status, result.score
        );
        match crate::controller::plan(
            self.controller.as_ref(),
            &self.controller_model,
            &prompt,
            "",
            crate::mode::Mode::Build,
        )
        .await
        {
            Ok(p) => p,
            Err(e) => format!("Correction planning failed: {e}. Raw issues:\n{issues_json}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn issues_brief(r: &VisualReviewResult) -> String {
    let mut s = format!("status={} score={}", r.status, r.score);
    for i in &r.issues {
        s.push_str(&format!(
            "\n  - [{}] {} / {}: {}",
            i.severity, i.category, i.component, i.description
        ));
    }
    s
}

/// Stable signature of the issue set for repeated-feedback detection (spec §19).
fn issue_signature(r: &VisualReviewResult) -> String {
    let mut parts: Vec<String> = r
        .issues
        .iter()
        .map(|i| {
            format!(
                "{}|{}|{}|{}",
                i.severity.to_lowercase(),
                i.category.to_lowercase(),
                i.component.to_lowercase(),
                i.description.to_lowercase()
            )
        })
        .collect();
    parts.sort();
    parts.join(" ; ")
}

/// Extract the first balanced JSON object from free text and parse it into a review result.
/// Missing fields default; a non-`approved` status is assumed when absent.
fn parse_review(text: &str) -> VisualReviewResult {
    // Use the shared string-aware balanced-JSON scanner so a `{` inside a string literal
    // (e.g. `{"description":"use {curly} here"}`) is not mis-parsed.
    let json = crate::subagents::last_balanced_json_object(text).unwrap_or_default();
    match serde_json::from_str::<VisualReviewResult>(&json) {
        Ok(mut r) => {
            if r.status.is_empty() {
                r.status = "rejected".into();
            }
            r
        }
        Err(_) => VisualReviewResult {
            status: "rejected".into(),
            score: 0,
            issues: vec![VisualIssue {
                severity: "major".into(),
                category: "review".into(),
                component: "review".into(),
                description: format!(
                    "LLM 3 returned unparseable review: {}",
                    json.chars().take(300).collect::<String>()
                ),
                recommendation: "Retry visual review.".into(),
                relevant_file: None,
            }],
        },
    }
}

/// Scan for the outermost `{ ... }` block (balanced braces).
#[cfg(test)]
mod tests {
    use super::*;
    use aether_config::VisualAcceptanceConfig;

    fn accepted(status: &str, score: u32, issues: Vec<VisualIssue>) -> VisualReviewResult {
        VisualReviewResult { status: status.into(), score, issues }
    }

    #[test]
    fn frontend_detection() {
        assert!(is_frontend_task("Design me a beautiful landing page"));
        assert!(is_frontend_task("Build a modern SaaS dashboard"));
        assert!(is_frontend_task("Make the UI/UX better"));
        assert!(!is_frontend_task("fix the parser bug in main.rs"));
        assert!(!is_frontend_task("explain the diff algorithm"));
    }

    #[test]
    fn gate_requires_full_configuration() {
        let fe = FrontendConfig {
            capture_command: Some("shot {out}".into()),
            max_visual_iterations: 5,
            ..Default::default()
        };
        // configured + frontend task -> true
        assert!(should_run_visual_review("build a website", &Some("reviewer".into()), &fe));
        // no reviewer model -> false
        assert!(!should_run_visual_review("build a website", &None, &fe));
        // no capture command -> false
        let fe2 = FrontendConfig { max_visual_iterations: 5, ..Default::default() };
        assert!(!should_run_visual_review("build a website", &Some("reviewer".into()), &fe2));
        // non-frontend task, not forced -> false
        assert!(!should_run_visual_review("fix bug", &Some("reviewer".into()), &fe));
        // force overrides task detection
        let fe3 = FrontendConfig { capture_command: Some("x".into()), force: true, max_visual_iterations: 5, ..Default::default() };
        assert!(should_run_visual_review("fix bug", &Some("reviewer".into()), &fe3));
    }

    #[test]
    fn acceptance_policy_not_numeric_only() {
        let acc = VisualAcceptanceConfig { require_no_critical: true, require_no_major: false, min_score: None };
        // approved, no issues -> ok
        assert!(VisualReviewPolicy::evaluate(&accepted("approved", 95, vec![]), &acc));
        // approved but a critical issue -> rejected by policy
        let crit = accepted("approved", 95, vec![VisualIssue {
            severity: "critical".into(), category: "responsive".into(), component: "nav".into(),
            description: "overlap".into(), recommendation: "fix".into(), relevant_file: None,
        }]);
        assert!(!VisualReviewPolicy::evaluate(&crit, &acc));
        // rejected status -> never approved
        assert!(!VisualReviewPolicy::evaluate(&accepted("rejected", 99, vec![]), &acc));
        // min_score respected
        let acc2 = VisualAcceptanceConfig { require_no_critical: true, require_no_major: false, min_score: Some(90) };
        assert!(!VisualReviewPolicy::evaluate(&accepted("approved", 80, vec![]), &acc2));
    }

    #[test]
    fn acceptance_policy_require_no_major() {
        let acc = VisualAcceptanceConfig { require_no_critical: true, require_no_major: true, min_score: None };
        // approved, only minor issues -> ok
        let minor = accepted("approved", 90, vec![VisualIssue {
            severity: "minor".into(), category: "spacing".into(), component: "p".into(),
            description: "tiny".into(), recommendation: "r".into(), relevant_file: None,
        }]);
        assert!(VisualReviewPolicy::evaluate(&minor, &acc));
        // approved, has a major issue -> rejected
        let major = accepted("approved", 90, vec![VisualIssue {
            severity: "major".into(), category: "hierarchy".into(), component: "h".into(),
            description: "big".into(), recommendation: "r".into(), relevant_file: None,
        }]);
        assert!(!VisualReviewPolicy::evaluate(&major, &acc));
    }

    #[test]
    fn parse_review_handles_braces_in_string() {
        // A `{` inside a JSON string literal must not split the object.
        let text = r#"{"status":"approved","score":90,"issues":[{"severity":"minor","category":"copy","component":"cta","description":"use {curly} here","recommendation":"avoid"}]}"#;
        let r = parse_review(text);
        assert_eq!(r.status, "approved");
        assert_eq!(r.issues.len(), 1);
        assert!(r.issues[0].description.contains("{curly}"));
    }

    #[test]
    fn parse_review_handles_free_text_and_bad_json() {
        let good = parse_review("Here is the review: {\"status\":\"approved\",\"score\":92,\"issues\":[]} thanks");
        assert_eq!(good.status, "approved");
        assert_eq!(good.score, 92);
        // missing status -> defaults to rejected
        let no_status = parse_review("{\"score\":50,\"issues\":[]}");
        assert_eq!(no_status.status, "rejected");
        // garbage -> rejected with a diagnostic issue
        let garbage = parse_review("no json here at all");
        assert_eq!(garbage.status, "rejected");
        assert!(!garbage.issues.is_empty());
    }

    #[test]
    fn issue_signature_is_order_independent() {
        let a = accepted("rejected", 10, vec![
            VisualIssue { severity: "major".into(), category: "x".into(), component: "c".into(), description: "B".into(), recommendation: "".into(), relevant_file: None },
            VisualIssue { severity: "minor".into(), category: "y".into(), component: "d".into(), description: "A".into(), recommendation: "".into(), relevant_file: None },
        ]);
        let b = accepted("rejected", 10, vec![
            VisualIssue { severity: "minor".into(), category: "y".into(), component: "d".into(), description: "A".into(), recommendation: "".into(), relevant_file: None },
            VisualIssue { severity: "major".into(), category: "x".into(), component: "c".into(), description: "B".into(), recommendation: "".into(), relevant_file: None },
        ]);
        assert_eq!(issue_signature(&a), issue_signature(&b));
    }

    #[test]
    fn state_labels_roundtrip() {
        for s in [
            VisualReviewState::Idle, VisualReviewState::FrontendReady, VisualReviewState::VisualReview,
            VisualReviewState::VisualApproved, VisualReviewState::VisualEscalated, VisualReviewState::Complete,
        ] {
            assert!(!s.label().is_empty());
        }
    }
}
