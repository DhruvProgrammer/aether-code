//! Configuration loading for `aether` (spec §25).
//! No API keys are stored here — only the env var name to read.

use serde::Deserialize;
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "dft_controller")]
    pub controller_model: String,
    #[serde(default = "dft_executor")]
    pub executor_model: String,
    #[serde(default = "dft_max_iter")]
    pub max_iterations: u32,
    /// Outer closed-loop budget: how many plan→execute→verify→replan cycles the
    /// engineering loop may run before the circuit breaker hard-stops (spec: loop engineering).
    #[serde(default = "dft_loop_budget")]
    pub loop_budget: u32,
    #[serde(default = "dft_policy")]
    pub routing_policy: String,
    /// Optional cheaper model key for trivial tasks (spec §8 cost routing).
    #[serde(default)]
    pub cheap_model: Option<String>,
    /// Endpoint used when running in local mode (`--local`), e.g. a local OpenAI-compatible server.
    #[serde(default = "dft_local_endpoint")]
    pub local_endpoint: String,
    /// LLM 3 — VISUAL FRONTEND REVIEWER (spec: 3-LLM visual engineering). Optional multimodal
    /// model key from `models`. When `None`, the visual-review loop is disabled and the system
    /// degrades gracefully to normal frontend development.
    #[serde(default)]
    pub reviewer_model: Option<String>,
}
fn dft_controller() -> String { "controller".into() }
fn dft_executor() -> String { "executor".into() }
fn dft_max_iter() -> u32 { 30 }
fn dft_loop_budget() -> u32 { 3 }
fn dft_policy() -> String { "balanced".into() }
fn dft_local_endpoint() -> String { "http://127.0.0.1:11434/v1".into() }

impl Default for AgentConfig {
    fn default() -> Self {
        AgentConfig {
            controller_model: dft_controller(),
            executor_model: dft_executor(),
            max_iterations: dft_max_iter(),
            loop_budget: dft_loop_budget(),
            routing_policy: dft_policy(),
            cheap_model: None,
            local_endpoint: dft_local_endpoint(),
            reviewer_model: None,
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

#[derive(Debug, Clone, Deserialize)]
pub struct ModelConfig {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub api_key_env: String,
    #[serde(default)]
    pub extra_body: Option<serde_json::Value>,
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
}
