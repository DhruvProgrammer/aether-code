//! Plugin trait + manifest.
//!
//! A [`Plugin`] is a typed unit that registers one or more hook handlers. The
//! runtime instantiates plugins once at startup (or on demand), then calls the
//! `register` method which gives back a manifest of hook handlers. Handlers are
//! stored in the global [`Registry`](crate::Registry) and invoked whenever an
//! extension point fires.

use async_trait::async_trait;

use crate::hooks::{
    AgentCompleteInput, AgentCompleteOutput, AgentSpawnHookInput, AgentSpawnHookOutput,
    ContextCompactInput, ContextCompactOutput, ModelRequestInput, ModelRequestOutput,
    ModelResponseInput, ModelResponseOutput, SessionEndInput, SessionEndOutput, SessionStartInput,
    SessionStartOutput, SnapshotCreateInput, SnapshotCreateOutput, SnapshotRestoreInput,
    SnapshotRestoreOutput, ToolExecuteInput, ToolExecuteOutput,
};

/// Identity + metadata for a registered plugin.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    /// Lower priority runs first. Built-in core hooks use priority 0; user
    /// plugins should use positive priorities (e.g. 100). Negative priorities
    /// are reserved for system-level plugins.
    pub priority: i32,
}

/// A typed plugin. Implement this trait to extend AETHER.
///
/// The runtime calls `register(&mut Registry)` once after constructing the
/// plugin. Inside `register` the plugin installs whichever hook handlers it
/// cares about via `Registry::on_before_model_request(...)`, etc. Everything
/// not registered is a no-op.
///
/// Plugins should be cheap to construct (no I/O in the constructor); if you
/// need to do async work, use the `OnSessionStart` hook.
#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    fn info(&self) -> &PluginInfo;

    async fn register(self: Box<Self>, registry: &mut crate::Registry) -> anyhow::Result<()>;

    // ---- Default no-op hook handlers. Override only what you need. ----

    async fn before_model_request(
        &self,
        _input: &mut ModelRequestInput,
        _output: &mut ModelRequestOutput,
    ) -> crate::hooks::HookOutcome {
        crate::hooks::HookOutcome::Continue
    }

    async fn after_model_response(
        &self,
        _input: &ModelResponseInput,
        _output: &mut ModelResponseOutput,
    ) -> crate::hooks::HookOutcome {
        crate::hooks::HookOutcome::Continue
    }

    async fn before_tool_execute(
        &self,
        _input: &mut ToolExecuteInput,
        _output: &mut ToolExecuteOutput,
    ) -> crate::hooks::HookOutcome {
        crate::hooks::HookOutcome::Continue
    }

    async fn after_tool_execute(
        &self,
        _input: &ToolExecuteInput,
        _output: &mut ToolExecuteOutput,
    ) -> crate::hooks::HookOutcome {
        crate::hooks::HookOutcome::Continue
    }

    async fn before_context_compact(
        &self,
        _input: &mut ContextCompactInput,
        _output: &mut ContextCompactOutput,
    ) -> crate::hooks::HookOutcome {
        crate::hooks::HookOutcome::Continue
    }

    async fn after_context_compact(
        &self,
        _input: &ContextCompactInput,
        _output: &mut ContextCompactOutput,
    ) -> crate::hooks::HookOutcome {
        crate::hooks::HookOutcome::Continue
    }

    async fn on_agent_spawn(
        &self,
        _input: &AgentSpawnHookInput,
        _output: &mut AgentSpawnHookOutput,
    ) -> crate::hooks::HookOutcome {
        crate::hooks::HookOutcome::Continue
    }

    async fn on_agent_complete(
        &self,
        _input: &AgentCompleteInput,
        _output: &mut AgentCompleteOutput,
    ) -> crate::hooks::HookOutcome {
        crate::hooks::HookOutcome::Continue
    }

    async fn on_snapshot_create(
        &self,
        _input: &SnapshotCreateInput,
        _output: &mut SnapshotCreateOutput,
    ) -> crate::hooks::HookOutcome {
        crate::hooks::HookOutcome::Continue
    }

    async fn on_snapshot_restore(
        &self,
        _input: &SnapshotRestoreInput,
        _output: &mut SnapshotRestoreOutput,
    ) -> crate::hooks::HookOutcome {
        crate::hooks::HookOutcome::Continue
    }

    async fn on_session_start(
        &self,
        _input: &SessionStartInput,
        _output: &mut SessionStartOutput,
    ) -> crate::hooks::HookOutcome {
        crate::hooks::HookOutcome::Continue
    }

    async fn on_session_end(
        &self,
        _input: &SessionEndInput,
        _output: &mut SessionEndOutput,
    ) -> crate::hooks::HookOutcome {
        crate::hooks::HookOutcome::Continue
    }
}
