//! Configuration loading for `aether` (spec §25).
//! No API keys are stored here — only the env var name to read.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub permissions: PermissionsConfig,
    #[serde(default)]
    pub context: ContextConfig,
    #[serde(default)]
    pub models: HashMap<String, ModelConfig>,
    #[serde(default)]
    pub display: DisplayConfig,
    #[serde(default)]
    pub subagents: SubagentsConfig,
    #[serde(default)]
    pub mcp: McpConfig,
    /// Frontend visual-engineering subsystem (spec: 3-LLM visual review). Controls the
    /// optional LLM 3 screenshot/review loop and its acceptance policy.
    #[serde(default)]
    pub frontend: FrontendConfig,
    /// Appearance settings (background image, opacity, on/off).
    #[serde(default)]
    pub appearance: AppearanceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceConfig {
    /// Master switch for the desktop background image.
    #[serde(default = "dft_bg_enabled")]
    pub background_enabled: bool,
    /// Background image opacity, 0..=100. Default tuned for a dark coding env.
    #[serde(default = "dft_bg_opacity")]
    pub background_opacity: u8,
    /// Resolved path to the background image file. When `None`, the desktop app
    /// falls back to the bundled default background shipped under `resources/`.
    #[serde(default)]
    pub background_image: Option<String>,
    /// Display mode: "fill" (cover), "fit" (contain), "stretch", "center".
    #[serde(default = "dft_bg_mode")]
    pub background_mode: String,
}
fn dft_bg_enabled() -> bool { true }
fn dft_bg_opacity() -> u8 { 60 }
fn dft_bg_mode() -> String { "fill".into() }

impl Default for AppearanceConfig {
    fn default() -> Self {
        AppearanceConfig {
            background_enabled: dft_bg_enabled(),
            background_opacity: dft_bg_opacity(),
            background_image: None,
            background_mode: dft_bg_mode(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    /// Model 1 — Big Executor (spec §3). Primary key into the `[models]` map. Required.
    /// Responsible for code execution, implementation, tool usage, coding, testing,
    /// debugging and the actual engineering work.
    #[serde(default = "dft_executor")]
    pub executor_model: String,
    /// Model 2 — Small Controller (spec §3). Optional key into `[models]`. When `None`
    /// or absent the existing/default orchestration system continues functioning.
    /// Responsible for planning, task decomposition, routing, design planning, correction
    /// planning, and deciding what happens next.
    #[serde(default = "dft_controller")]
    pub controller_model: String,
    /// Model 3 — Visual Frontend Reviewer (spec §3). Optional multimodal model key. When
    /// `None` the visual-review loop is disabled and the system degrades gracefully to
    /// normal frontend development. Activated only at visual-review checkpoints.
    #[serde(default)]
    pub reviewer_model: Option<String>,
    /// Legacy alias preserved for backward compatibility; mirrors `executor_model`.
    #[serde(default, skip_serializing)]
    pub model1: Option<String>,
    /// Legacy alias preserved for backward compatibility; mirrors `controller_model`.
    #[serde(default, skip_serializing)]
    pub model2: Option<String>,
    /// Legacy alias preserved for backward compatibility; mirrors `reviewer_model`.
    #[serde(default, skip_serializing)]
    pub model3: Option<String>,
    #[serde(default = "dft_max_iter")]
    pub max_iterations: u32,
    /// Outer closed-loop budget: how many plan→execute→verify→replan cycles the
    /// engineering loop may run before the circuit breaker hard-stops (spec: loop engineering).
    #[serde(default = "dft_loop_budget")]
    pub loop_budget: u32,
    /// Endpoint used when running in local mode (`--local`), e.g. a local OpenAI-compatible server.
    #[serde(default = "dft_local_endpoint")]
    pub local_endpoint: String,
}
fn dft_controller() -> String { "controller".into() }
fn dft_executor() -> String { "executor".into() }
fn dft_max_iter() -> u32 { 30 }
fn dft_loop_budget() -> u32 { 3 }
fn dft_local_endpoint() -> String { "http://127.0.0.1:11434/v1".into() }

impl Default for AgentConfig {
    fn default() -> Self {
        let exec = dft_executor();
        let ctrl = dft_controller();
        AgentConfig {
            executor_model: exec.clone(),
            controller_model: ctrl.clone(),
            reviewer_model: None,
            model1: Some(exec),
            model2: Some(ctrl),
            model3: None,
            max_iterations: dft_max_iter(),
            loop_budget: dft_loop_budget(),
            local_endpoint: dft_local_endpoint(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryConfig {
    #[serde(default = "dft_true")]
    pub enabled: bool,
    #[serde(default = "dft_embedded")]
    pub backend: String,
    #[serde(default = "dft_redb")]
    pub graph_store: String,
    #[serde(default = "dft_usearch")]
    pub vector_store: String,
    #[serde(default = "dft_emb")]
    pub embedding_provider: String,
    #[serde(default = "dft_topk")]
    pub memory_top_k: usize,
    #[serde(default = "dft_mind_path")]
    pub path: String,
    #[serde(default = "dft_false")]
    pub auto_extract: bool,
}
fn dft_true() -> bool { true }
fn dft_false() -> bool { false }
fn dft_embedded() -> String { "embedded".into() }
fn dft_redb() -> String { "redb".into() }
fn dft_usearch() -> String { "usearch".into() }
fn dft_emb() -> String { "openai_compatible".into() }
fn dft_topk() -> usize { 8 }
fn dft_mind_path() -> String { "~/.aether/mind.redb".into() }

impl Default for MemoryConfig {
    fn default() -> Self {
        MemoryConfig {
            enabled: dft_true(),
            backend: dft_embedded(),
            graph_store: dft_redb(),
            vector_store: dft_usearch(),
            embedding_provider: dft_emb(),
            memory_top_k: dft_topk(),
            path: dft_mind_path(),
            auto_extract: dft_false(),
        }
    }
}

/// Expand a leading `~` to the user's home directory.
pub fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~") {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(rest.trim_start_matches('/'))
    } else {
        PathBuf::from(s)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PermissionsConfig {
    #[serde(default = "dft_allow")]
    pub read: String,
    #[serde(default = "dft_allow")]
    pub edit: String,
    #[serde(default = "dft_ask")]
    pub bash: String,
    #[serde(default = "dft_ask")]
    pub delete: String,
    #[serde(default = "dft_ask")]
    pub git_commit: String,
    #[serde(default = "dft_ask")]
    pub network: String,
}
fn dft_allow() -> String { "allow".into() }
fn dft_ask() -> String { "ask".into() }

impl Default for PermissionsConfig {
    fn default() -> Self {
        PermissionsConfig {
            read: dft_allow(),
            edit: dft_allow(),
            bash: dft_ask(),
            delete: dft_ask(),
            git_commit: dft_ask(),
            network: dft_ask(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContextConfig {
    #[serde(default = "dft_ctx")]
    pub max_tokens: u32,
}
fn dft_ctx() -> u32 { 128000 }

impl Default for ContextConfig {
    fn default() -> Self { ContextConfig { max_tokens: dft_ctx() } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub api_key_env: String,
    #[serde(default)]
    pub headers: Option<serde_json::Value>,
    #[serde(default)]
    pub extra_body: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Provider registry (v0.17 redesign)
// ---------------------------------------------------------------------------

/// A provider entry in the registry. Credentials are stored once per provider;
/// models reference the provider by ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEntry {
    /// Unique provider ID (user-chosen slug, e.g. "nvidia", "openrouter").
    pub id: String,
    /// Human-readable display name.
    #[serde(default)]
    pub display_name: String,
    /// Protocol adapter: currently only "openai_compatible".
    #[serde(default = "dft_protocol")]
    pub protocol: String,
    /// Base URL for the provider's API.
    pub base_url: String,
    /// Environment variable name holding the API key (never the key itself).
    pub api_key_env: String,
    /// Optional extra headers sent with every request.
    #[serde(default)]
    pub headers: Option<serde_json::Value>,
    /// Optional extra body fields merged into every request.
    #[serde(default)]
    pub extra_body: Option<serde_json::Value>,
    /// Models registered under this provider.
    #[serde(default)]
    pub models: Vec<ModelEntry>,
}
fn dft_protocol() -> String { "openai_compatible".into() }

/// A model entry under a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    /// The model ID sent to the API (e.g. "gpt-4o", "meta/llama-3.1-70b").
    pub id: String,
    /// Human-readable display name.
    #[serde(default)]
    pub display_name: String,
    /// Whether the model supports vision/image input.
    #[serde(default)]
    pub vision: bool,
    /// Whether the model supports tool calling.
    #[serde(default = "dft_true")]
    pub tool_calling: bool,
    /// Whether the model supports streaming.
    #[serde(default = "dft_true")]
    pub streaming: bool,
    /// Context window size in tokens (when known).
    #[serde(default)]
    pub context_window: Option<u32>,
    /// Maximum output tokens (when known).
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
}

/// Per-session role assignment: which provider/model performs each AETHER role.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoleAssignments {
    /// LLM 1 — Big Executor. Required.
    pub executor: Option<RoleBinding>,
    /// LLM 2 — Small Controller. Required.
    pub controller: Option<RoleBinding>,
    /// LLM 3 — Visual Frontend Reviewer. Optional.
    pub reviewer: Option<RoleBinding>,
}

/// A single role → provider/model binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleBinding {
    /// Provider ID from the registry.
    pub provider_id: String,
    /// Model ID within that provider.
    pub model_id: String,
}

impl ProviderEntry {
    /// Resolve a model entry by ID.
    pub fn model(&self, model_id: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.id == model_id)
    }

    /// Build a `ModelConfig` compatible with the legacy `[models]` map for a
    /// given model under this provider. Used by the gateway adapter.
    pub fn to_model_config(&self, model_id: &str) -> Option<ModelConfig> {
        self.model(model_id).map(|_m| ModelConfig {
            provider: self.protocol.clone(),
            base_url: self.base_url.clone(),
            model: model_id.to_string(),
            api_key_env: self.api_key_env.clone(),
            headers: self.headers.clone(),
            extra_body: self.extra_body.clone(),
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DisplayConfig {
    #[serde(default = "dft_light")]
    pub theme: String,
    #[serde(default = "dft_accent")]
    pub accent: String,
    #[serde(default)]
    pub emoji: bool,
}
fn dft_light() -> String { "light".into() }
fn dft_accent() -> String { "still-blue".into() }

impl Default for DisplayConfig {
    fn default() -> Self {
        DisplayConfig { theme: dft_light(), accent: dft_accent(), emoji: false }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubagentsConfig {
    /// Enable the multi-agent verification pipeline (Explorer + Tester + Reviewer, + Security
    /// Reviewer on risk). When off, the loop still runs but skips the agent handoff pass.
    #[serde(default = "dft_true")]
    pub enabled: bool,
}

impl Default for SubagentsConfig {
    fn default() -> Self {
        SubagentsConfig { enabled: dft_true() }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

/// Frontend visual-engineering configuration (spec: 3-LLM visual review).
///
/// LLM 3 (the visual reviewer) is optional. When `reviewer_model` (in `AgentConfig`) is unset,
/// no `capture_command` is configured, or the task is not a frontend task, the system degrades
/// gracefully and the visual loop never starts.
#[derive(Debug, Clone, Deserialize)]
pub struct FrontendConfig {
    /// Shell command that renders the frontend and writes a screenshot PNG. The tokens
    /// `{out}` (target png path) and `{cwd}` (project dir) are substituted before running.
    /// When `None`, the visual-review loop cannot capture and is skipped.
    #[serde(default)]
    pub capture_command: Option<String>,
    /// Optional shell command that starts a local preview/dev server before capture (and is
    /// killed afterwards). The token `{cwd}` is substituted.
    #[serde(default)]
    pub preview_command: Option<String>,
    /// Hard cap on visual-review iterations (loop protection). 0 disables the loop.
    #[serde(default = "dft_max_visual")]
    pub max_visual_iterations: u32,
    /// Force the visual-review loop on for every task (useful for testing). Auto-detected otherwise.
    #[serde(default)]
    pub force: bool,
    /// Explicit acceptance policy (spec §12). Score is supporting evidence only.
    #[serde(default)]
    pub acceptance: VisualAcceptanceConfig,
}

fn dft_max_visual() -> u32 { 5 }

impl Default for FrontendConfig {
    fn default() -> Self {
        FrontendConfig {
            capture_command: None,
            preview_command: None,
            max_visual_iterations: dft_max_visual(),
            force: false,
            acceptance: VisualAcceptanceConfig::default(),
        }
    }
}

/// Explicit visual-acceptance contract (spec §12). Approval is NOT purely numeric.
#[derive(Debug, Clone, Deserialize)]
pub struct VisualAcceptanceConfig {
    /// Reject unless there are zero `critical` issues.
    #[serde(default = "dft_true")]
    pub require_no_critical: bool,
    /// Reject unless there are zero `major` issues.
    #[serde(default = "dft_false")]
    pub require_no_major: bool,
    /// Optional minimum score (0-100); supporting evidence only.
    #[serde(default)]
    pub min_score: Option<u32>,
}

impl Default for VisualAcceptanceConfig {
    fn default() -> Self {
        VisualAcceptanceConfig {
            require_no_critical: dft_true(),
            require_no_major: dft_false(),
            min_score: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
}

impl Config {
    /// Load config; falls back to defaults if the path does not exist.
    pub fn load(path: Option<PathBuf>) -> Result<Config, ConfigError> {
        let path = path.unwrap_or_else(Config::default_path);
        if !path.exists() {
            return Ok(Config::default());
        }
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn default_path() -> PathBuf {
        let mut p = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        p.push(".aether");
        p.push("config.toml");
        p
    }

    /// The `~/.aether` data directory (config, sessions, memory).
    pub fn default_dir() -> PathBuf {
        let mut p = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        p.push(".aether");
        p
    }

    pub fn model(&self, key: &str) -> Option<&ModelConfig> {
        self.models.get(key)
    }

    /// Resolve Model 1 (Big Executor) — required.
    pub fn model1(&self) -> Option<&ModelConfig> {
        let key = self.agent.model1.as_deref().unwrap_or(&self.agent.executor_model);
        self.models.get(key)
    }

    /// Resolve Model 2 (Small Controller) — optional.
    pub fn model2(&self) -> Option<&ModelConfig> {
        match self.agent.model2.as_deref() {
            Some(k) if !k.is_empty() => self.models.get(k),
            _ => {
                let k = &self.agent.controller_model;
                if k.is_empty() { None } else { self.models.get(k) }
            }
        }
    }

    /// Resolve Model 3 (Visual Reviewer) — optional.
    pub fn model3(&self) -> Option<&ModelConfig> {
        match self.agent.model3.as_deref() {
            Some(k) if !k.is_empty() => self.models.get(k),
            _ => self.agent.reviewer_model.as_deref().and_then(|k| self.models.get(k)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Config {
        let toml = r#"
            [agent]
            executor_model = "executor"
            controller_model = "controller"
            reviewer_model = "reviewer"
            model1 = "executor"
            model2 = "controller"
            model3 = "reviewer"

            [models.executor]
            provider = "openai_compatible"
            base_url = "https://api.example.com/v1"
            model = "big-coder"
            api_key_env = "BIG_KEY"

            [models.controller]
            provider = "openai_compatible"
            base_url = "https://controller.example.com/v1"
            model = "small-ctrl"
            api_key_env = "CTRL_KEY"

            [models.reviewer]
            provider = "openai_compatible"
            base_url = "https://reviewer.example.com/v1"
            model = "vision-pro"
            api_key_env = "REV_KEY"

            [appearance]
            background_enabled = true
            background_opacity = 42
        "#;
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn appearance_defaults_are_tasteful() {
        let cfg = Config::default();
        assert!(cfg.appearance.background_enabled);
        assert_eq!(cfg.appearance.background_opacity, 60);
        assert_eq!(cfg.appearance.background_image, None);
        assert_eq!(cfg.appearance.background_mode, "fill");
    }

    #[test]
    fn model1_resolves_to_required_executor() {
        let cfg = sample();
        let m1 = cfg.model1().expect("model1 must resolve");
        assert_eq!(m1.model, "big-coder");
        assert_eq!(m1.api_key_env, "BIG_KEY");
    }

    #[test]
    fn model2_resolves_to_optional_controller() {
        let cfg = sample();
        let m2 = cfg.model2().expect("model2 must resolve when configured");
        assert_eq!(m2.model, "small-ctrl");
    }

    #[test]
    fn model3_resolves_to_optional_reviewer() {
        let cfg = sample();
        let m3 = cfg.model3().expect("model3 must resolve when configured");
        assert_eq!(m3.model, "vision-pro");
    }

    #[test]
    fn model2_missing_is_none_not_error() {
        let mut cfg = sample();
        cfg.agent.model2 = None;
        cfg.agent.controller_model = "missing-key".into();
        // Either way: returns None (Model 2 is optional).
        assert!(cfg.model2().is_none());
    }

    #[test]
    fn model3_missing_is_none_not_error() {
        let mut cfg = sample();
        cfg.agent.model3 = None;
        cfg.agent.reviewer_model = None;
        assert!(cfg.model3().is_none());
    }

    #[test]
    fn appearance_roundtrip() {
        let cfg = sample();
        let s = toml::to_string(&cfg.appearance).unwrap();
        let back: AppearanceConfig = toml::from_str(&s).unwrap();
        assert!(back.background_enabled);
        assert_eq!(back.background_opacity, 42);
    }
}
