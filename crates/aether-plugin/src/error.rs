//! Plugin error types.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("plugin not registered: {0}")]
    NotRegistered(String),

    #[error("plugin panicked during hook {hook}: {message}")]
    HookPanic { hook: String, message: String },

    #[error("plugin '{plugin}' returned invalid output for hook {hook}: {message}")]
    InvalidOutput {
        plugin: String,
        hook: String,
        message: String,
    },

    #[error("hook '{0}' aborted: {1}")]
    Aborted(String, String),
}

pub type PluginResult<T> = Result<T, PluginError>;
