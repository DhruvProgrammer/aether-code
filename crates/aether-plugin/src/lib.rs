//! `aether-plugin` — typed middleware hook system for AETHER.
//!
//! Inspired by OpenCode's plugin model (each hook is an `(input, output) => Promise<void>`
//! transform registered on a global `Plugin` service) but redesigned for AETHER's
//! Rust async architecture and multi-agent runtime.
//!
//! # Concepts
//!
//! * [`Plugin`] — a typed unit that registers one or more hook handlers. Plugins
//!   implement only the hooks they care about; everything else is a no-op.
//! * [`Hook`] — a named, strongly-typed middleware slot. Hooks have a fixed
//!   `Input` and `Output` type and chain ordered by [`Plugin::priority`].
//! * [`Registry`] — global hook registry; holds plugins and runs the chain
//!   when an extension point fires an event.
//! * [`Event`] — runtime-emitted typed event (snapshot created, agent spawned,
//!   tool executed, model response received, etc.). Subscribers via [`Registry::subscribe`].
//!
//! # Built-in hooks
//!
//! | Hook name                     | Purpose                                              |
//! |-------------------------------|------------------------------------------------------|
//! | `BeforeModelRequest`          | Mutate / veto a completion request before it leaves  |
//! | `AfterModelResponse`          | Observe / transform a completion response            |
//! | `BeforeToolExecute`           | Permission + audit before a tool runs                |
//! | `AfterToolExecute`            | Observe / transform a tool result                    |
//! | `BeforeContextCompact`        | Decide if compaction should run / pin segments       |
//! | `AfterContextCompact`         | Observe compaction outcomes                          |
//! | `OnAgentSpawn`                | Audit + mutate agent definitions on spawn           |
//! | `OnAgentComplete`             | Audit + collect agent results                        |
//! | `OnSnapshotCreate`            | Snapshot audit / metadata enrichment                 |
//! | `OnSnapshotRestore`           | Pre-restore gate                                     |
//! | `OnSessionStart`              | Session-level init                                   |
//! | `OnSessionEnd`                | Session-level teardown                               |
//!
//! All hooks are async, cancel-safe, and may short-circuit by returning
//! `HookOutcome::Halt { reason }` from `Before*` hooks.

pub mod error;
pub mod hooks;
pub mod plugin;
pub mod registry;
pub mod event;
pub mod bus;

pub use error::{PluginError, PluginResult};
pub use hooks::{
    AgentCompleteInput, AgentCompleteOutput, AgentSpawnHookInput, AgentSpawnHookOutput,
    ContextCompactInput, ContextCompactOutput, Hook, HookOutcome, ModelRequestInput, ModelRequestOutput,
    ModelResponseInput, ModelResponseOutput, SessionEndInput, SessionEndOutput, SessionStartInput,
    SessionStartOutput, SnapshotCreateInput, SnapshotCreateOutput, SnapshotRestoreInput, SnapshotRestoreOutput,
    ToolExecuteInput, ToolExecuteOutput,
};
pub use plugin::{Plugin, PluginInfo};
pub use registry::Registry;
pub use event::{Event, EventKind, EventSubscriber};
pub use bus::{global as global_registry, publish as publish_event, subscribe as subscribe_events};
