//! Global plugin registry. Holds plugins, dispatches hook chains.

use std::sync::Arc;

use parking_lot::RwLock;

use crate::error::{PluginError, PluginResult};
use crate::event::{Event, EventKind, EventSubscriber};
use crate::hooks::{
    AgentCompleteInput, AgentCompleteOutput, AgentSpawnHookInput, AgentSpawnHookOutput,
    ContextCompactInput, ContextCompactOutput, Hook, HookOutcome, ModelRequestInput, ModelRequestOutput,
    ModelResponseInput, ModelResponseOutput, SessionEndInput, SessionEndOutput, SessionStartInput,
    SessionStartOutput, SnapshotCreateInput, SnapshotCreateOutput, SnapshotRestoreInput,
    SnapshotRestoreOutput, ToolExecuteInput, ToolExecuteOutput,
};
use crate::plugin::{Plugin, PluginInfo};

type PluginEntry = Arc<dyn Plugin>;

// Per-hook chain storage: priority-ordered plugins implementing the hook.
struct HookChain<H: ?Sized, I, O> {
    handlers: Vec<(PluginInfo, Arc<H>)>,
    _phantom: std::marker::PhantomData<fn() -> (I, O)>,
}

impl<H: ?Sized, I, O> Default for HookChain<H, I, O> {
    fn default() -> Self {
        Self {
            handlers: Vec::new(),
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<H: Hook<I, O> + ?Sized, I, O> HookChain<H, I, O> {
    fn install(&mut self, info: PluginInfo, handler: Arc<H>) {
        // Insert in priority order (ascending; lower priority runs first).
        let pos = self
            .handlers
            .binary_search_by_key(&info.priority, |(i, _)| i.priority)
            .unwrap_or_else(|e| e);
        self.handlers.insert(pos, (info, handler));
    }

    async fn run(&self, input: &mut I, output: &mut O) -> PluginResult<()> {
        for (info, h) in &self.handlers {
            match h.run(input, output).await {
                HookOutcome::Continue => continue,
                HookOutcome::Halt { reason } => {
                    return Err(PluginError::Aborted(info.id.clone(), reason));
                }
            }
        }
        Ok(())
    }

    fn plugins(&self) -> Vec<String> {
        self.handlers.iter().map(|(i, _)| i.id.clone()).collect()
    }
}

/// Wraps a `dyn Plugin` as a `Hook` by dispatching to the right default method.
macro_rules! hook_wrapper {
    ($wrapper:ident, $trait:ident, $input:ty, $output:ty, $method:ident) => {
        pub struct $wrapper {
            plugin: PluginEntry,
        }

        impl $wrapper {
            pub fn new(plugin: PluginEntry) -> Self {
                Self { plugin }
            }
        }

        #[async_trait::async_trait]
        impl Hook<$input, $output> for $wrapper {
            async fn run(
                &self,
                input: &mut $input,
                output: &mut $output,
            ) -> HookOutcome {
                self.plugin.$method(input, output).await
            }
        }
    };
}

hook_wrapper!(
    BeforeModelRequestHook,
    Hook,
    ModelRequestInput,
    ModelRequestOutput,
    before_model_request
);
hook_wrapper!(
    AfterModelResponseHook,
    Hook,
    ModelResponseInput,
    ModelResponseOutput,
    after_model_response
);
hook_wrapper!(
    BeforeToolExecuteHook,
    Hook,
    ToolExecuteInput,
    ToolExecuteOutput,
    before_tool_execute
);
hook_wrapper!(
    AfterToolExecuteHook,
    Hook,
    ToolExecuteInput,
    ToolExecuteOutput,
    after_tool_execute
);
hook_wrapper!(
    BeforeContextCompactHook,
    Hook,
    ContextCompactInput,
    ContextCompactOutput,
    before_context_compact
);
hook_wrapper!(
    AfterContextCompactHook,
    Hook,
    ContextCompactInput,
    ContextCompactOutput,
    after_context_compact
);
hook_wrapper!(
    AgentSpawnHook,
    Hook,
    AgentSpawnHookInput,
    AgentSpawnHookOutput,
    on_agent_spawn
);
hook_wrapper!(
    AgentCompleteHook,
    Hook,
    AgentCompleteInput,
    AgentCompleteOutput,
    on_agent_complete
);
hook_wrapper!(
    SnapshotCreateHook,
    Hook,
    SnapshotCreateInput,
    SnapshotCreateOutput,
    on_snapshot_create
);
hook_wrapper!(
    SnapshotRestoreHook,
    Hook,
    SnapshotRestoreInput,
    SnapshotRestoreOutput,
    on_snapshot_restore
);
hook_wrapper!(
    SessionStartHook,
    Hook,
    SessionStartInput,
    SessionStartOutput,
    on_session_start
);
hook_wrapper!(
    SessionEndHook,
    Hook,
    SessionEndInput,
    SessionEndOutput,
    on_session_end
);

/// The global plugin + event registry.
pub struct Registry {
    plugins: RwLock<Vec<PluginEntry>>,

    before_model: RwLock<HookChain<BeforeModelRequestHook, ModelRequestInput, ModelRequestOutput>>,
    after_model: RwLock<HookChain<AfterModelResponseHook, ModelResponseInput, ModelResponseOutput>>,
    before_tool: RwLock<HookChain<BeforeToolExecuteHook, ToolExecuteInput, ToolExecuteOutput>>,
    after_tool: RwLock<HookChain<AfterToolExecuteHook, ToolExecuteInput, ToolExecuteOutput>>,
    before_compact: RwLock<HookChain<BeforeContextCompactHook, ContextCompactInput, ContextCompactOutput>>,
    after_compact: RwLock<HookChain<AfterContextCompactHook, ContextCompactInput, ContextCompactOutput>>,
    on_spawn: RwLock<HookChain<AgentSpawnHook, AgentSpawnHookInput, AgentSpawnHookOutput>>,
    on_complete: RwLock<HookChain<AgentCompleteHook, AgentCompleteInput, AgentCompleteOutput>>,
    on_snap_create: RwLock<HookChain<SnapshotCreateHook, SnapshotCreateInput, SnapshotCreateOutput>>,
    on_snap_restore: RwLock<HookChain<SnapshotRestoreHook, SnapshotRestoreInput, SnapshotRestoreOutput>>,
    on_session_start: RwLock<HookChain<SessionStartHook, SessionStartInput, SessionStartOutput>>,
    on_session_end: RwLock<HookChain<SessionEndHook, SessionEndInput, SessionEndOutput>>,

    subscribers: RwLock<Vec<Arc<dyn EventSubscriber>>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    pub fn new() -> Self {
        Self {
            plugins: RwLock::new(Vec::new()),
            before_model: RwLock::new(HookChain::default()),
            after_model: RwLock::new(HookChain::default()),
            before_tool: RwLock::new(HookChain::default()),
            after_tool: RwLock::new(HookChain::default()),
            before_compact: RwLock::new(HookChain::default()),
            after_compact: RwLock::new(HookChain::default()),
            on_spawn: RwLock::new(HookChain::default()),
            on_complete: RwLock::new(HookChain::default()),
            on_snap_create: RwLock::new(HookChain::default()),
            on_snap_restore: RwLock::new(HookChain::default()),
            on_session_start: RwLock::new(HookChain::default()),
            on_session_end: RwLock::new(HookChain::default()),
            subscribers: RwLock::new(Vec::new()),
        }
    }

    // ---- Registration ----------------------------------------------------

    /// Register a plugin. Idempotent — re-registering with the same id is a no-op.
    pub async fn register(&self, plugin: Box<dyn Plugin>) -> PluginResult<()> {
        let info = plugin.info().clone();
        let arc: Arc<dyn Plugin> = Arc::from(plugin);
        {
            let mut ps = self.plugins.write();
            if ps.iter().any(|p| p.info().id == info.id) {
                return Ok(());
            }
            ps.push(arc.clone());
        }
        // Install into the appropriate chains.
        self.before_model
            .write()
            .install(info.clone(), Arc::new(BeforeModelRequestHook::new(arc.clone())));
        self.after_model
            .write()
            .install(info.clone(), Arc::new(AfterModelResponseHook::new(arc.clone())));
        self.before_tool
            .write()
            .install(info.clone(), Arc::new(BeforeToolExecuteHook::new(arc.clone())));
        self.after_tool
            .write()
            .install(info.clone(), Arc::new(AfterToolExecuteHook::new(arc.clone())));
        self.before_compact
            .write()
            .install(info.clone(), Arc::new(BeforeContextCompactHook::new(arc.clone())));
        self.after_compact
            .write()
            .install(info.clone(), Arc::new(AfterContextCompactHook::new(arc.clone())));
        self.on_spawn
            .write()
            .install(info.clone(), Arc::new(AgentSpawnHook::new(arc.clone())));
        self.on_complete
            .write()
            .install(info.clone(), Arc::new(AgentCompleteHook::new(arc.clone())));
        self.on_snap_create
            .write()
            .install(info.clone(), Arc::new(SnapshotCreateHook::new(arc.clone())));
        self.on_snap_restore
            .write()
            .install(info.clone(), Arc::new(SnapshotRestoreHook::new(arc.clone())));
        self.on_session_start
            .write()
            .install(info.clone(), Arc::new(SessionStartHook::new(arc.clone())));
        self.on_session_end
            .write()
            .install(info.clone(), Arc::new(SessionEndHook::new(arc.clone())));
        Ok(())
    }

    pub fn unregister(&self, id: &str) {
        let mut ps = self.plugins.write();
        ps.retain(|p| p.info().id != id);
        // We do not bother removing from chains; chains are tiny.
    }

    pub fn plugins(&self) -> Vec<PluginInfo> {
        self.plugins.read().iter().map(|p| p.info().clone()).collect()
    }

    // ---- Hook dispatch ---------------------------------------------------

    pub async fn before_model_request(
        &self,
        input: &mut ModelRequestInput,
        output: &mut ModelRequestOutput,
    ) -> PluginResult<()> {
        self.before_model.read().run(input, output).await
    }

    pub async fn after_model_response(
        &self,
        input: &ModelResponseInput,
        output: &mut ModelResponseOutput,
    ) -> PluginResult<()> {
        // Need a mutable copy for the chain.
        let mut i = input.clone();
        self.after_model.read().run(&mut i, output).await
    }

    pub async fn before_tool_execute(
        &self,
        input: &mut ToolExecuteInput,
        output: &mut ToolExecuteOutput,
    ) -> PluginResult<()> {
        self.before_tool.read().run(input, output).await
    }

    pub async fn after_tool_execute(
        &self,
        input: &ToolExecuteInput,
        output: &mut ToolExecuteOutput,
    ) -> PluginResult<()> {
        let mut i = input.clone();
        self.after_tool.read().run(&mut i, output).await
    }

    pub async fn before_context_compact(
        &self,
        input: &mut ContextCompactInput,
        output: &mut ContextCompactOutput,
    ) -> PluginResult<()> {
        self.before_compact.read().run(input, output).await
    }

    pub async fn after_context_compact(
        &self,
        input: &ContextCompactInput,
        output: &mut ContextCompactOutput,
    ) -> PluginResult<()> {
        let mut i = input.clone();
        self.after_compact.read().run(&mut i, output).await
    }

    pub async fn on_agent_spawn(
        &self,
        input: &AgentSpawnHookInput,
        output: &mut AgentSpawnHookOutput,
    ) -> PluginResult<()> {
        let mut i = input.clone();
        self.on_spawn.read().run(&mut i, output).await
    }

    pub async fn on_agent_complete(
        &self,
        input: &AgentCompleteInput,
        output: &mut AgentCompleteOutput,
    ) -> PluginResult<()> {
        let mut i = input.clone();
        self.on_complete.read().run(&mut i, output).await
    }

    pub async fn on_snapshot_create(
        &self,
        input: &SnapshotCreateInput,
        output: &mut SnapshotCreateOutput,
    ) -> PluginResult<()> {
        let mut i = input.clone();
        self.on_snap_create.read().run(&mut i, output).await
    }

    pub async fn on_snapshot_restore(
        &self,
        input: &SnapshotRestoreInput,
        output: &mut SnapshotRestoreOutput,
    ) -> PluginResult<()> {
        let mut i = input.clone();
        self.on_snap_restore.read().run(&mut i, output).await
    }

    pub async fn on_session_start(
        &self,
        input: &SessionStartInput,
        output: &mut SessionStartOutput,
    ) -> PluginResult<()> {
        let mut i = input.clone();
        self.on_session_start.read().run(&mut i, output).await
    }

    pub async fn on_session_end(
        &self,
        input: &SessionEndInput,
        output: &mut SessionEndOutput,
    ) -> PluginResult<()> {
        let mut i = input.clone();
        self.on_session_end.read().run(&mut i, output).await
    }

    // ---- Inspection helpers ----------------------------------------------

    pub fn hook_plugin_ids(&self, hook: &str) -> Vec<String> {
        match hook {
            "before_model_request" => self.before_model.read().plugins(),
            "after_model_response" => self.after_model.read().plugins(),
            "before_tool_execute" => self.before_tool.read().plugins(),
            "after_tool_execute" => self.after_tool.read().plugins(),
            "before_context_compact" => self.before_compact.read().plugins(),
            "after_context_compact" => self.after_compact.read().plugins(),
            "on_agent_spawn" => self.on_spawn.read().plugins(),
            "on_agent_complete" => self.on_complete.read().plugins(),
            "on_snapshot_create" => self.on_snap_create.read().plugins(),
            "on_snapshot_restore" => self.on_snap_restore.read().plugins(),
            "on_session_start" => self.on_session_start.read().plugins(),
            "on_session_end" => self.on_session_end.read().plugins(),
            _ => Vec::new(),
        }
    }

    // ---- Event subscribers -----------------------------------------------

    pub fn subscribe(&self, sub: Arc<dyn EventSubscriber>) {
        self.subscribers.write().push(sub);
    }

    pub async fn publish(&self, kind: EventKind, payload: serde_json::Value) {
        let event = Event { kind, payload };
        let subs: Vec<_> = self.subscribers.read().iter().cloned().collect();
        for sub in subs {
            // Errors from one subscriber must not stop others.
            if let Err(e) = sub.on_event(&event).await {
                tracing::warn!(error = %e, "event subscriber failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::Plugin;

    struct TestPlugin {
        info: PluginInfo,
    }

    #[async_trait::async_trait]
    impl Plugin for TestPlugin {
        fn info(&self) -> &PluginInfo {
            &self.info
        }
        async fn register(
            self: Box<Self>,
            _registry: &mut crate::Registry,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn empty_registry_no_handlers() {
        let r = Registry::new();
        let mut input = ModelRequestInput {
            model: "m".into(),
            system: String::new(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            agent_id: None,
        };
        let mut output = ModelRequestOutput::default();
        assert!(r.before_model_request(&mut input, &mut output).await.is_ok());
    }

    #[tokio::test]
    async fn registration_is_idempotent() {
        let r = Registry::new();
        let p = TestPlugin {
            info: PluginInfo {
                id: "t".into(),
                name: "t".into(),
                version: "0.1".into(),
                description: "".into(),
                priority: 100,
            },
        };
        r.register(Box::new(p)).await.unwrap();
        r.register(Box::new(TestPlugin {
            info: PluginInfo {
                id: "t".into(),
                name: "t".into(),
                version: "0.1".into(),
                description: "".into(),
                priority: 100,
            },
        }))
        .await
        .unwrap();
        assert_eq!(r.plugins().len(), 1);
    }

    #[tokio::test]
    async fn hook_chain_order_respects_priority() {
        let r = Registry::new();
        // Lower priority runs first.
        r.register(Box::new(TestPlugin {
            info: PluginInfo {
                id: "low".into(),
                name: "low".into(),
                version: "0".into(),
                description: "".into(),
                priority: 10,
            },
        }))
        .await
        .unwrap();
        r.register(Box::new(TestPlugin {
            info: PluginInfo {
                id: "high".into(),
                name: "high".into(),
                version: "0".into(),
                description: "".into(),
                priority: 100,
            },
        }))
        .await
        .unwrap();
        let ids = r.hook_plugin_ids("before_model_request");
        assert_eq!(ids, vec!["low".to_string(), "high".to_string()]);
    }
}
